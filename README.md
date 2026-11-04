# lrcli

Control Adobe Lightroom Classic from the command line, so scripts and AI agents
can browse a catalog, apply edits, and look at rendered previews of the results.

> **Status:** early development. Nothing works yet.

## How it works

Lightroom Classic has no CLI or headless mode, so lrcli has two parts:

- **A Lua plugin** that runs inside Lightroom Classic and listens on localhost
  sockets.
- **A command-line client** (TypeScript/Node) that sends it commands as
  newline-delimited JSON and prints the results.

Lightroom Classic must be running with a catalog open. Its window can be
hidden.

## Requirements

- macOS (Windows support is planned)
- Adobe Lightroom Classic
- Node.js 22 or newer

## License

[MIT](LICENSE)
