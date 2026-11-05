# lrcli

Control Adobe Lightroom Classic from the command line, so scripts and AI agents
can browse a catalog, apply edits, and look at rendered previews of the results.

> **Status:** early development. Status, plugin install, catalog queries,
> previews and the first editing commands (`apply`, `preset`, `tag`, `rate`,
> `flag`, `label`, `undo`) exist.

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
plus a menu item), so no manual Plug-in Manager step is needed. `lrcli start`
launches Lightroom in the background; it briefly comes to the front once
(the plugin can't start otherwise) and focus returns to your app. If something
goes wrong, open **File > Plug-in Extras > lrcli Status…** to see the
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
the plugin), `plugin.log`, `broker.sock`, `broker.log`, `config.json`,
`history.jsonl` (undo history).

## Commands

```sh
lrcli start [--catalog PATH] [--wait SECS]                     # launch Lightroom and wait for the plugin
lrcli stop [--wait SECS]                                       # quit Lightroom (never force-kills)
lrcli restart [--catalog PATH]                                 # stop, then start
lrcli search --rating '>=4' --flag picked --since 2024-01-01   # table of matching photos
lrcli search --keyword portrait --ids | lrcli select --photos - # ids in, selection out
lrcli selection                                                # what is selected in Lightroom
lrcli info --photo 42 --develop                                # metadata, EXIF, develop settings
lrcli settings --panel basic --photo 42                        # develop setting names by panel, with ranges and current values
lrcli preview --photo 42 --size 1200                           # JPEG preview with current edits, plus stats
lrcli apply --photo 42 Exposure2012+=0.5 Contrast2012=15 --preview  # edit develop settings, then render a preview
lrcli apply --photo 42 Exposure2012+=0.5 --compare             # before/after image (left = before, right = after) plus stats
lrcli compare before.jpg after.jpg [--out PATH]                # side-by-side JPEG of two images, plus stats
lrcli presets                                                  # develop presets by folder
lrcli preset "Warm Contrast" --amount 60 --photo 42            # apply a develop preset
lrcli develop auto-tone auto-wb --photo 42                     # Develop actions: also as-shot-wb, reset-all, --reset KEY,...
lrcli module develop                                           # switch Lightroom module (library, develop, map, ...)
lrcli tag add "Places > Paris" portrait --photo 42             # add keywords (tag remove to remove)
lrcli rate 4 --photo 42                                        # stars; also flag picked|rejected|none, label red|...|none
lrcli undo                                                     # revert the last recorded edit (--steps N, --dry-run)
lrcli history                                                  # what undo would revert
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

Editing commands (`apply`, `preset`, `develop`, `tag`, `rate`, `flag`, `label`) take the same
`--photo`/`--photos -` targets and default to the current selection. Each writes
all its photos in one step, which shows up in Lightroom's Edit > Undo menu, and
records the replaced values in `~/.lrcli/history.jsonl` so `lrcli undo` can
restore them (for the catalog that is open).

`develop` runs Lightroom's own Develop actions (auto tone, auto white balance,
as-shot white balance, resets). Lightroom only runs them in the Develop module,
so the command switches to Develop, selects each photo in turn, and afterwards
restores the previous module and selection. It works with Lightroom in the
background and records the changes for `undo` like the other edits.

`settings` lists the develop setting names `apply` accepts, grouped by panel,
with valid ranges (for a given `--photo`: that photo's ranges and current
values; non-raw files use `IncrementalTemperature`/`IncrementalTint` instead of
`Temperature`/`Tint`). `--compare` on `apply`, `preset` and `develop` renders a preview
before and after the write and saves a side-by-side image per photo (left =
before, right = after) next to the after preview, with mean luminance and
clipping before and after. `compare` does the same for two existing JPEGs.

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
