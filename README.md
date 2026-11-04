# lrcli

Control Adobe Lightroom Classic from the command line, so scripts and AI agents
can browse a catalog, apply edits, and look at rendered previews of the results.

> **Status:** early development. `lrcli status`, `lrcli plugin install` and the
> read-only catalog commands (`search`, `select`, `selection`, `info`) exist;
> editing commands are coming.

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

Lightroom Classic must be running with a catalog open (`lrcli start` launches
it). Its window can be hidden.

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

Then start (or restart) Lightroom Classic so it loads the plugin, and check the
connection:

```sh
lrcli restart
lrcli status
```

The plugin forces Lightroom to initialise it at startup (`LrForceInitPlugin`
plus a menu item), so no manual Plug-in Manager step is needed. If something
goes wrong, open **Library > Plug-in Extras > lrcli Status…** to see the
plugin's recent log.

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

## Commands

```sh
lrcli start [--catalog PATH] [--wait SECS]                     # launch Lightroom and wait for the plugin
lrcli stop [--wait SECS]                                       # quit Lightroom (never force-kills)
lrcli restart [--catalog PATH]                                 # stop, then start
lrcli search --rating '>=4' --flag picked --since 2024-01-01   # table of matching photos
lrcli search --keyword portrait --ids | lrcli select --photos - # ids in, selection out
lrcli selection                                                # what is selected in Lightroom
lrcli info --photo 42 --develop                                # metadata, EXIF, develop settings
lrcli preview --photo 42 --size 1200                           # JPEG preview with current edits, plus stats
```

`start`, `stop` and `restart` control the Lightroom Classic app (macOS only).
`start` does nothing if the plugin already answers; with `--catalog` it fails if
a different catalog is open (use `restart --catalog`). The first time, macOS
asks whether your terminal app may control Lightroom Classic (Automation
permission); allow it.

Photos are identified by the catalog's local id (the `ID` column of `search`).
`info` and `select` take repeated `--photo <id>`, or `--photos -` to read ids
from stdin (one per line, or the JSON from `search --json`). `info` without ids
acts on the current selection. `preview` writes JPEGs under
`~/.lrcli/previews/` and prints their paths with luminance and clipping stats;
`--fast` uses Lightroom's cached preview, which can be stale after edits. Run `lrcli <command> --help` for all options.

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
