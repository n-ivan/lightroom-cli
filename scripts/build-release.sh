#!/bin/sh
# Builds dist/lrcli-macos-universal.tar.gz (+ .sha256): a universal, ad-hoc
# signed lrcli with LICENSE and README.md.
set -eu

cd "$(dirname "$0")/.."

targets="aarch64-apple-darwin x86_64-apple-darwin"
installed=$(rustup target list --installed)
missing=""
for target in $targets; do
  case "$installed" in
    *"$target"*) ;;
    *) missing="$missing $target" ;;
  esac
done
if [ -n "$missing" ]; then
  echo "error: missing rustup targets; run: rustup target add$missing" >&2
  exit 1
fi

for target in $targets; do
  cargo build --release --target "$target"
done

rm -rf dist
mkdir -p dist
lipo -create \
  target/aarch64-apple-darwin/release/lrcli \
  target/x86_64-apple-darwin/release/lrcli \
  -output dist/lrcli
chmod 755 dist/lrcli
codesign --force -s - dist/lrcli

archs=$(lipo -archs dist/lrcli)
case "$archs" in
  *arm64*x86_64* | *x86_64*arm64*) ;;
  *) echo "error: dist/lrcli has architectures '$archs', expected arm64 and x86_64" >&2; exit 1 ;;
esac
codesign -v dist/lrcli

archive=lrcli-macos-universal.tar.gz
cp LICENSE README.md dist/
tar -czf "dist/$archive" -C dist lrcli LICENSE README.md
(cd dist && shasum -a 256 "$archive" > "$archive.sha256")

echo "Built dist/$archive ($archs)"
