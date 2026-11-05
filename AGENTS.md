# lrcli

Rust CLI plus Lightroom Classic Lua plugin. The plugin runs inside LrC and
serves newline-delimited JSON over two localhost sockets (command and reply,
because LrSocket is one-directional). The plugin spawns one long-lived broker
(`lrcli broker`, hidden subcommand) that holds a persistent connection to it;
CLI invocations talk only to the broker over `~/.lrcli/broker.sock`. Never
connect the CLI straight to the plugin: reconnecting per invocation deadlocks
Lightroom.

## Layout

- `plugin/lrcli.lrdevplugin/`: Lua 5.1 plugin. `json.lua`, `Dispatch.lua` and
  `Version.lua` are pure Lua (no `Lr*` imports) so luajit can test them.
- `SearchCriteria.lua` (pure): validates search filters and builds the
  `findPhotos` searchDesc; all SDK criteria names live in one table at its top.
- `Photos.lua` (pure): builds summary/info objects from already-fetched metadata.
- `Targets.lua` (pure, catalog injected): resolves explicit ids or the selection.
- `CatalogHandlers.lua`: the catalog commands (`search`, `select`, `selection`,
  `info`). Merged in by `Handlers.lua`.
- `DevelopKeys.lua`: GENERATED name-to-type table of develop settings; never
  edit it, run `scripts/gen-develop-keys.py` (reads the untracked SDK docs).
- `Writes.lua` (pure): argument validation, develop value computation, keyword
  tree lookup, preset lookup, the write-gate wrapper. `WriteHandlers.lua`: the
  catalog write commands (`apply`, `presets`, `preset`, `tag`, `rate`, `flag`,
  `label`, `restore`); merged in by `Handlers.lua`.
- `DevelopHandlers.lua`: the `develop` and `module` commands (drive
  `LrDevelopController` in the Develop module); `DevelopActions.lua` (pure):
  their argument validation.
- `DevelopRanges.lua`: GENERATED min/max per develop key, `raw` and `nonRaw`;
  never edit it. Regenerate: run `lrcli raw develop_ranges '{"photos":[<id>]}'`
  on a raw photo and on a non-raw one (e.g. TIFF), save the JSON replies to
  `scripts/data/develop-ranges-raw.json` / `develop-ranges-nonraw.json`, then run
  `scripts/gen-develop-ranges.py` and `scripts/gen-develop-panels.py`. It probes
  documented keys plus the photo's own keys; pass `"names": [...]` for keys a
  photo only has in some states (e.g. `GrayMixer*` on black-and-white photos,
  `LensBlur*`), or they get no range and drop out of `settings`.
- `DevelopPanels.lua`: GENERATED panel-to-keys list for `settings`; never edit
  it, run `scripts/gen-develop-panels.py` (reads the untracked SDK docs and the
  key/range data above). `PanelSettings.lua` (pure): panel lookup and the
  raw/non-raw range listing; the `settings` handler is in `CatalogHandlers.lua`.
- `Preview.lua` (pure): `preview` arg validation, export settings table, result
  assembly.
- `PreviewHandler.lua`: the `preview` command (export session or cached
  thumbnails); merged in by `Handlers.lua`.
- `Export.lua` (pure): `export` arg validation, export settings table.
  `ExportHandler.lua`: the `export` command. `Rendering.lua`: the shared
  `LrExportSession` run and photo info lookup (used by `preview` too).
- `StatusMenuItem.lua`: the File > Plug-in Extras > "lrcli Status…" item;
  it also makes `LrForceInitPlugin` effective (see gotchas).
- `plugin/tests/run.lua`: luajit tests, run from the repo root.
- `src/lib.rs` and its modules are the reusable library (a future MCP server
  should depend on it): `client.rs` (CLI to broker), `broker.rs`,
  `plugin_link.rs` (broker to plugin, id multiplexing), `protocol.rs`,
  `paths.rs`, `error.rs`, `log.rs`, `commands/`, `platform/` (OS-specific
  paths, app launch/quit; `LRCLI_FAKE_APP=running|stopped` fakes them in tests), `stats.rs` (JPEG luminance and clipping stats; no printing), `compare.rs`
  (side-by-side before/after JPEG; used by `compare` and `--compare`).
- `build.rs` embeds `plugin/lrcli.lrdevplugin/` into the binary
  (`src/plugin_files.rs`); `lrcli plugin install` writes it out, so the binary
  never needs the checkout.
- `scripts/bump-version.sh` (versions), `scripts/build-release.sh` (universal
  macOS tarball in `dist/`, needs both rustup darwin targets; `make release`),
  `install.sh` (the curl installer; `tests/install_script.rs` runs it against a
  `file://` release), `.github/workflows/` (`ci.yml`: `make check`;
  `release.yml`: on `v*` tags builds and publishes the release).
- `src/main.rs` (clap, exit mapping) and `src/output.rs` (human/JSON printing)
  are the only places that print or exit.
- `tests/`: integration tests spawn the built binary. `tests/common/mod.rs`
  has the fake plugin (two TCP listeners emulating the LrSocket pair).
- `local/`: untracked scratch. The spec is `local/spec.md`.

## Lightroom SDK reference

Check every SDK call, metadata key and search criterion against the SDK docs
before using it; don't rely on memory or other projects. The SDK (Adobe's
Lightroom Classic SDK download) lives untracked in `local/sdk/`: API reference
in `API Reference/modules/*.html` (e.g. `LrCatalog.html` for `findPhotos`
criteria), guide in `Manual/` (searchable text in `guide.txt`, extracted with
`swift pdftext.swift <pdf>`; export setting keys are in chapter 3), examples in
`Sample Plugins/`. If it's missing,
ask the user for it. Anything the docs don't cover (e.g. `getPhotoByLocalId`, see gotchas)
counts as unsupported unless verified live.

## Lightroom SDK gotchas (verified on LrC 15.6, macOS)

- Use `LrTasks.pcall`, never `pcall`, around anything that can yield
  (`LrTasks.sleep`, `LrTasks.execute`, catalog calls). Plain `pcall` fails with
  "Yielding is not allowed within a C or metamethod call".
- LrSocket callbacks only record events; the server loop in `Server.lua` does
  all socket work. Reconnecting or rebinding right after a client disconnect
  deadlocked Lightroom's UI thread. Rebinding is only a recovery path: bump the
  generation first (callbacks fired by `close()` are then ignored), close, sleep
  0.1 s, bind.
- A send-mode LrSocket never notices its client leaving, and handles a
  half-closed peer badly, so the broker keeps its reply socket's write half
  open.
- `catalog:getTargetPhotos()` returns the whole filmstrip when nothing is
  selected; detect "nothing selected" with `catalog:getTargetPhoto() == nil`.
- Lightroom defers `LrInitPlugin` until the plugin is first used unless Info.lua
  sets `LrForceInitPlugin = true` and the plugin contributes a menu item; both
  are needed. Use `LrExportMenuItems` (File menu): a `LrLibraryMenuItems` item
  doesn't count when Lightroom opens in Develop or another module, and init
  silently never happens. Even then init waits until the
  window has been activated once: `lrcli start` launches with `open -g` and
  activates Lightroom for 0.2 s, then returns focus. A background launch alone
  never initialises the plugin.
- `catalog:getPhotoByLocalId(id)` is undocumented but present (LrC 15.6). For
  an unknown id it returns a placeholder LrPhoto, not nil; a real photo has a
  `uuid`. `Targets.finder` uses it and falls back to scanning `getAllPhotos()`
  if the method disappears.
- Lightroom indexes a plugin's `.lua` files at startup: Reload Plug-in picks up
  edits to existing files, but a newly added file fails with "Could not load
  toolkit script: <name>" until Lightroom restarts (use `lrcli restart`). On any init failure,
  `Init.lua` logs per-file compile and load errors to find the real cause.
- A reloaded plugin gets a fresh global environment, so a previous instance
  can't be reached through `_G`. The newest instance wins via
  `instanceStartedAt` in `ports.json`; older instances stop themselves.
- No `LrShutdownApp`/`LrShutdownPlugin` hooks: they made quitting hang on
  "performing shutdown tasks". Lightroom cancels the server's function context
  on quit, and the broker exits when its connection closes.
- `applyDevelopSettings` does not clamp: out-of-range values (Exposure2012=12)
  are stored, and a photo holding them renders as if none of its develop edits
  applied. `apply` therefore checks `DevelopRanges` (raw vs non-raw by
  `fileFormat`) and errors per photo; `restore` writes recorded values as-is.
- Undo restores recorded before-values with a conflict check; LrUndo isn't used
  because it can't be inspected and coalesces consecutive scripted edits (one
  undo reverted three develop runs across two photos).
- `LrDevelopController` acts on the current photo and only in the Develop
  module, so `develop` selects each photo (`setSelectedPhotos`), switches module
  (1 s sleep, 0.3 s per photo once there) and restores module and selection
  afterwards; no write gate is needed. "As shot" white balance has no
  controller call: `resetToDefault('Temperature')` and `('Tint')` restore the
  camera values but leave the label on Custom, `setValue('WhiteBalance',
  'As Shot')` does nothing, so a gate applies `WhiteBalance = 'As Shot'`.
- Controller changes show up in `getDevelopSettings` a moment late, so
  `develop` polls (0.1 s, up to 2 s) until they differ from the before-snapshot;
  an action can legitimately change nothing.
- Decoded JSON arrays carry json.lua's marker metatable and must reach the SDK
  as plain tables (`Writes.plain`); otherwise `withWriteAccessDo` hangs on commit.
- `LR_jpeg_quality` is 0..1, not 0..100. `LR_size_resizeType = 'longEdge'` is
  constrained by `LR_size_maxHeight`, so set both max values.
- The SDK guide's `LRtiff_compressionMethod` is a typo: Lightroom only honours
  `LR_tiff_compressionMethod`. Verify export settings by inspecting the output
  file, since Lightroom silently ignores unknown keys.
- Never export with `LR_collisionHandling = 'ask'`: Lightroom opens a dialog
  and the export blocks until someone answers it.
- Server commands run in their own task so the loop keeps ticking; only `ping`
  is answered while one runs (`Dispatch.nextIndex`).
- `catalog:withWriteAccessDo` returns "aborted" (not an error) when write access
  isn't granted within the timeout; `Writes.gate` turns that into `write_busy`.
  Errors thrown inside roll the whole gate back. Every write command makes one
  gate for all its photos.
- `colorNameForLabel` accepts only red, yellow, green, blue, purple, none; any
  other string silently gives a white label. Rating "none" is `nil`, not 0.
- The SDK has no keyword lookup by name: `Writes.planKeyword` walks
  `getKeywords()`/`getChildren()`. Keywords created in a gate are unreadable
  until it ends, so the chain is created and attached in the same gate.
- The `getDevelopSettings` docs list is incomplete (no Temperature, Tint,
  Texture) and has typos; the generator script patches both and says how.
- LrLogger's log location varies by version, so the plugin logs to
  `~/.lrcli/plugin.log`; the last lines also show in Plug-in Manager.

## Checks

`make check` = `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`, then Lua syntax checks and `plugin/tests/run.lua`. Requires
`luajit`. `make build` makes the release binary.

## Plugin install flavours and releases

- Release install: `lrcli plugin install` writes `lrcli.lrplugin/` (embedded
  plugin plus a copy of the binary in `bin/lrcli`); `Broker.lua` prefers that
  binary, so broker and plugin versions always match. Dev install:
  `lrcli plugin install --dev` symlinks `lrcli.lrdevplugin` and records the
  binary in `~/.lrcli/config.json`. Each install removes the other (same
  toolkit id would conflict).
- Every broker-using command compares the plugin's hello version with the CLI's
  and warns on stderr on mismatch (`status` also reports `versionMismatch`).
- Cutting a release: `scripts/bump-version.sh X.Y.Z`, `make check`, commit,
  `git tag vX.Y.Z`, push the commit and the tag. The workflow checks that the
  tag equals the Cargo version and publishes the tarball and `.sha256`.

## Conventions

- Exit codes: 0 ok, 1 command failed, 2 usage, 3 unreachable, 4 protocol/auth,
  5 timeout. Defined in `src/error.rs`.
- `--json` or `LRCLI_OUTPUT=json` prints the reply without `id`; errors are
  `{ok:false,error,message}` on stdout. Human mode prints errors to stderr.
- Error messages say what to do next.
- Library modules return values and `Error`s; they never print or exit.
- `--help` on every command carries examples; agents read it as documentation.
- Tests set `LRCLI_HOME` and `LRCLI_MODULES_DIR` to temp dirs; never touch the
  real `~/.lrcli` or Lightroom's Modules directory.
- Protocol changes must keep `Version.lua`, `Info.lua` and `Cargo.toml`
  versions in step.

## Comments

Comment only a non-obvious why, gotcha, or SDK quirk. No comments that restate
code, section banners, narration of changes, or TODO/step markers. No doc-comment
boilerplate. Prefer self-documenting names.
