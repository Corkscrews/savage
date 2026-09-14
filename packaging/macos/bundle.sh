#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
bin="$root/target/release/savage"
dest="${1:-$root/target/Savage.app}"

cargo build --release --manifest-path "$root/Cargo.toml"
"$root/packaging/macos/assemble-app.sh" "$bin" "$dest"

echo "Bundled $dest"
echo "Set as default SVG viewer: right-click an .svg → Get Info → Open with → Savage → Change All"
