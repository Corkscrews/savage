#!/bin/sh
set -eu

if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
  echo "usage: assemble-app.sh <binary> <Savage.app> [version]" >&2
  exit 0
fi

bin=${1:-}
dest=${2:-}
version=${3:-}

if [ -z "$bin" ] || [ -z "$dest" ]; then
  echo "usage: assemble-app.sh <binary> <Savage.app> [version]" >&2
  exit 1
fi

if [ ! -f "$bin" ]; then
  echo "missing binary: $bin" >&2
  exit 1
fi

root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
plist="$root/packaging/macos/Info.plist"

if [ -z "$version" ]; then
  version=$(awk -F\" '/^version = / { print $2; exit }' "$root/Cargo.toml")
fi

rm -rf "$dest"
mkdir -p "$dest/Contents/MacOS"
cp "$bin" "$dest/Contents/MacOS/savage"
chmod 755 "$dest/Contents/MacOS/savage"
cp "$plist" "$dest/Contents/Info.plist"
printf 'APPL????' > "$dest/Contents/PkgInfo"

if command -v /usr/libexec/PlistBuddy >/dev/null 2>&1; then
  /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$dest/Contents/Info.plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $version" "$dest/Contents/Info.plist"
fi

if command -v codesign >/dev/null 2>&1; then
  codesign --force --sign - "$dest" >/dev/null 2>&1 || true
fi
