#!/bin/sh
# Installs lrcli and its Lightroom Classic plugin.
#   curl -fsSL https://raw.githubusercontent.com/n-ivan/lightroom-cli/main/install.sh | sh
# Environment: LRCLI_VERSION (default latest), LRCLI_INSTALL_DIR (default
# ~/.local/bin), LRCLI_DOWNLOAD_BASE (default the GitHub releases URL).
set -eu

ARCHIVE=lrcli-macos-universal.tar.gz
BASE=${LRCLI_DOWNLOAD_BASE:-https://github.com/n-ivan/lightroom-cli/releases}
VERSION=${LRCLI_VERSION:-latest}
INSTALL_DIR=${LRCLI_INSTALL_DIR:-$HOME/.local/bin}

fail() {
  echo "error: $*" >&2
  exit 1
}

[ "$(uname -s)" = Darwin ] || fail "lrcli only supports macOS (Lightroom Classic plugin), found $(uname -s)."

case $VERSION in
  latest) url_base=$BASE/latest/download ;;
  *) url_base=$BASE/download/v${VERSION#v} ;;
esac

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT HUP INT TERM

echo "Downloading lrcli ($VERSION)..."
curl -fsSL "$url_base/$ARCHIVE" -o "$work/$ARCHIVE" || fail "could not download $url_base/$ARCHIVE"
curl -fsSL "$url_base/$ARCHIVE.sha256" -o "$work/$ARCHIVE.sha256" || fail "could not download $url_base/$ARCHIVE.sha256"

(cd "$work" && shasum -a 256 -c "$ARCHIVE.sha256" >/dev/null) || fail "checksum mismatch for $ARCHIVE; not installing."

tar -xzf "$work/$ARCHIVE" -C "$work" lrcli

mkdir -p "$INSTALL_DIR"
cp "$work/lrcli" "$INSTALL_DIR/lrcli.tmp"
chmod 755 "$INSTALL_DIR/lrcli.tmp"
mv -f "$INSTALL_DIR/lrcli.tmp" "$INSTALL_DIR/lrcli"
echo "Installed $INSTALL_DIR/lrcli"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo
    echo "$INSTALL_DIR is not on your PATH. For zsh, run:"
    echo "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.zshrc && source ~/.zshrc"
    echo
    ;;
esac

"$INSTALL_DIR/lrcli" plugin install

cat <<MSG

Next steps:
  lrcli restart     start (or restart) Lightroom Classic so it loads the plugin
  lrcli status      check the connection
  lrcli --help      list every command
MSG
