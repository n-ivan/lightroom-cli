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
- `DevelopRanges.lua`: GENERATED min/max per develop key, `raw` and `nonRaw`;
  never edit it. Regenerate: run `lrcli raw develop_ranges '{"photos":[<id>]}'`
  on a raw photo and on a non-raw one (e.g. TIFF), save the JSON replies to
  `scripts/data/develop-ranges-raw.json` / `develop-ranges-nonraw.json`, then run
  `scripts/gen-develop-ranges.py`.
- `Preview.lua` (pure): `preview` arg validation, export settings table, result
  assembly.
- `PreviewHandler.lua`: the `preview` command (export session or cached
  thumbnails); merged in by `Handlers.lua`.
- `StatusMenuItem.lua`: the File > Plug-in Extras > "lrcli Status…" item;
  it also makes `LrForceInitPlugin` effective (see gotchas).
- `plugin/tests/run.lua`: luajit tests, run from the repo root.
- `src/lib.rs` and its modules are the reusable library (a future MCP server
  should depend on it): `client.rs` (CLI to broker), `broker.rs`,
  `plugin_link.rs` (broker to plugin, id multiplexing), `protocol.rs`,
  `paths.rs`, `error.rs`, `log.rs`, `commands/`, `platform/` (OS-specific
  paths, app launch/quit; `LRCLI_FAKE_APP=running|stopped` fakes them in tests), `stats.rs` (JPEG luminance and clipping stats; no printing).
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
ask the user for it. Anything the docs don't cover (e.g. `getPhotoByLocalId`)
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
- `LrUndo` has only `canUndo/canRedo/undo/redo`; there is no way to see what
  will be undone, and consecutive `withWriteAccessDo` calls without user
  interaction coalesce into ONE undo step (so a second undo can revert an
  unrelated action). `undo_native` therefore undoes once, waits 0.3 s, verifies
  the recorded before-state (absent keys must be nil) and the previous lrcli
  step's after-state, and redoes if either differs; `lrcli undo` then falls
  back to `restore`, which cannot clear keys that had no value before.
- Decoded JSON arrays carry json.lua's marker metatable and must reach the SDK
  as plain tables (`Writes.plain`); otherwise `withWriteAccessDo` hangs on commit.
- `LR_jpeg_quality` is 0..1, not 0..100. `LR_size_resizeType = 'longEdge'` is
  constrained by `LR_size_maxHeight`, so set both max values.
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
