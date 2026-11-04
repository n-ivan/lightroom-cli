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
- `plugin/tests/run.lua`: luajit tests, run from the repo root.
- `src/lib.rs` and its modules are the reusable library (a future MCP server
  should depend on it): `client.rs` (CLI to broker), `broker.rs`,
  `plugin_link.rs` (broker to plugin, id multiplexing), `protocol.rs`,
  `paths.rs`, `error.rs`, `log.rs`, `commands/`, `platform/` (OS-specific
  paths).
- `src/main.rs` (clap, exit mapping) and `src/output.rs` (human/JSON printing)
  are the only places that print or exit.
- `tests/`: integration tests spawn the built binary. `tests/common/mod.rs`
  has the fake plugin (two TCP listeners emulating the LrSocket pair).
- `local/`: untracked scratch. The spec is `local/spec.md`.

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
- Lightroom defers `LrInitPlugin` until the plugin is first used (opening
  Plug-in Manager works); launching or focusing the app is not enough.
- A reloaded plugin gets a fresh global environment, so a previous instance
  can't be reached through `_G`. The newest instance wins via
  `instanceStartedAt` in `ports.json`; older instances stop themselves.
- No `LrShutdownApp`/`LrShutdownPlugin` hooks: they made quitting hang on
  "performing shutdown tasks". Lightroom cancels the server's function context
  on quit, and the broker exits when its connection closes.
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
