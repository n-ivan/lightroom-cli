#!/bin/sh
# Usage: scripts/bump-version.sh NEW_VERSION (for example 0.2.0)
set -eu

[ $# -eq 1 ] || { echo "usage: $0 NEW_VERSION" >&2; exit 2; }
version=$1
case $version in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "error: version must look like MAJOR.MINOR.PATCH, got '$version'" >&2; exit 2 ;;
esac
major=${version%%.*}
rest=${version#*.}
minor=${rest%%.*}
patch=${rest#*.}
case "$major$minor$patch" in
  *[!0-9]*) echo "error: version must be numeric MAJOR.MINOR.PATCH, got '$version'" >&2; exit 2 ;;
esac

cd "$(dirname "$0")/.."
plugin=plugin/lrcli.lrdevplugin

sed -i.bak "s/^version = \".*\"/version = \"$version\"/" Cargo.toml
sed -i.bak "s/PLUGIN = '[^']*'/PLUGIN = '$version'/" "$plugin/Version.lua"
sed -i.bak "s/VERSION = { major = [0-9]*, minor = [0-9]*, revision = [0-9]* }/VERSION = { major = $major, minor = $minor, revision = $patch }/" "$plugin/Info.lua"
rm -f Cargo.toml.bak "$plugin/Version.lua.bak" "$plugin/Info.lua.bak"

# Refreshes the lrcli entry in Cargo.lock without touching dependencies.
cargo update --offline -p lrcli >/dev/null 2>&1 || cargo check --offline >/dev/null

echo "Bumped to $version. Review with git diff, then run make check."
