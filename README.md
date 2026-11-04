# lrcli

Control Adobe Lightroom Classic from the command line, so scripts and AI agents
can browse a catalog, apply edits, and look at rendered previews of the results.

> **Status:** early development. `lrcli status` and `lrcli plugin install` work;
> catalog commands are coming.

## How it works

Lightroom Classic has no CLI or headless mode, so lrcli has three parts: a Lua
plugin that runs inside Lightroom Classic and serves commands on localhost
sockets, a long-lived broker process that the plugin starts and that holds the
only connection to it, and the `lrcli` command, which talks to the broker over a
Unix socket. The broker exists because Lightroom's socket API deadlocks when
clients connect and disconnect repeatedly. Everything is one Rust binary; the
broker is its hidden `broker` subcommand.

```
lrcli status ──unix socket ~/.lrcli/broker.sock──▶ lrcli broker ──TCP (persistent)──▶ LrC plugin
```

Lightroom Classic must be running with a catalog open. Its window can be
hidden.

## Requirements

- macOS (Windows support is planned)
- Adobe Lightroom Classic
- Rust (edition 2024 toolchain) to build from source

## Install from source

```sh
cargo build --release
cargo install --path .     # or put target/release/lrcli on your PATH
lrcli plugin install
```

Then restart Lightroom Classic. Lightroom defers plugin start-up until the
plugin is first used, so open **File > Plug-in Manager** and select lrcli once;
its status panel shows the plugin log. Then check the connection:

```sh
lrcli status
```

`plugin install` symlinks `plugin/lrcli.lrdevplugin` into Lightroom's Modules
directory and records this binary's path in `~/.lrcli/config.json` so the
plugin can start the broker. Run it again if you move the binary.

## Usage

```sh
lrcli --help                 # every command, with examples
lrcli status
lrcli --json status          # or LRCLI_OUTPUT=json
lrcli --timeout 60 status    # seconds per request (default 30)
```

With `--json`, the reply is printed on stdout without its `id`, and errors are
`{"ok":false,"error":"<code>","message":"<text>"}`. In human mode errors go to
stderr.

State lives in `~/.lrcli` (override with `LRCLI_HOME`): `ports.json` (written by
the plugin), `plugin.log`, `broker.sock`, `broker.log`, `config.json`.

### Exit codes

| Code | Meaning                                                              |
| ---- | -------------------------------------------------------------------- |
| 0    | Success                                                              |
| 1    | The command failed (plugin replied `ok:false`, or a local error)     |
| 2    | Usage error                                                          |
| 3    | Lightroom Classic unreachable (no broker, refused, or connection lost) |
| 4    | Protocol or authentication mismatch between lrcli and the plugin    |
| 5    | Timed out waiting for a reply                                        |

## Development

```sh
make check    # cargo fmt --check, clippy -D warnings, cargo test, Lua syntax + tests
make build    # release build
```

`make check` needs `luajit` for the plugin's syntax check and tests.
