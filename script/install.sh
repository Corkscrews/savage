#!/bin/sh
set -eu

if [ "$(uname -s)" != "Darwin" ]; then
  echo "install.sh only supports macOS (installs into /Applications)." >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found in PATH." >&2
  exit 1
fi

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
bin="$root/target/release/savage"
staged="$root/target/Savage.app"
dest="${SAVAGE_INSTALL_DIR:-/Applications}/Savage.app"
assemble="$root/packaging/macos/assemble-app.sh"

echo "Building savage (release)..."
cargo build --release --manifest-path "$root/Cargo.toml"

"$assemble" "$bin" "$staged"

echo "Installing $dest"
if [ -w "$(dirname "$dest")" ]; then
  rm -rf "$dest"
  ditto "$staged" "$dest"
else
  echo "Need administrator rights to write $(dirname "$dest")"
  sudo rm -rf "$dest"
  sudo ditto "$staged" "$dest"
fi

lsregister="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
if [ -x "$lsregister" ]; then
  "$lsregister" -f "$dest"
fi

echo "Installed $dest"
echo "Set as default SVG viewer: right-click an .svg → Get Info → Open with → Savage → Change All"
