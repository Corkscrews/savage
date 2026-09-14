#!/bin/sh
# Build a universal Savage.app zip for Homebrew Cask / GitHub Releases.
set -eu

if [ "$(uname -s)" != "Darwin" ]; then
  echo "package-macos.sh must run on macOS." >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found in PATH." >&2
  exit 1
fi

if ! command -v lipo >/dev/null 2>&1; then
  echo "lipo not found; install Xcode command line tools." >&2
  exit 1
fi

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
version=$(awk -F\" '/^version = / { print $2; exit }' "$root/Cargo.toml")
dist="$root/dist"
app="$dist/Savage.app"
zip="$dist/Savage-${version}-macos.zip"
assemble="$root/packaging/macos/assemble-app.sh"
cask="$root/Casks/savage.rb"

github_repo() {
  url=$(git -C "$root" remote get-url origin 2>/dev/null || true)
  [ -n "$url" ] || return 0
  url=${url%.git}
  url=${url%/}
  case "$url" in
    git@github.com:*) printf '%s\n' "${url#git@github.com:}" ;;
    https://github.com/*) printf '%s\n' "${url#https://github.com/}" ;;
    http://github.com/*) printf '%s\n' "${url#http://github.com/}" ;;
    ssh://git@github.com/*) printf '%s\n' "${url#ssh://git@github.com/}" ;;
  esac
}

export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"

if command -v rustup >/dev/null 2>&1; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
fi

echo "Building savage ${version} (universal darwin)..."
cargo build --release --locked --manifest-path "$root/Cargo.toml" --target aarch64-apple-darwin
cargo build --release --locked --manifest-path "$root/Cargo.toml" --target x86_64-apple-darwin

universal=$(mktemp)
trap 'rm -f "$universal"' EXIT
lipo -create \
  "$root/target/aarch64-apple-darwin/release/savage" \
  "$root/target/x86_64-apple-darwin/release/savage" \
  -output "$universal"

rm -rf "$app"
mkdir -p "$dist"
"$assemble" "$universal" "$app" "$version"

rm -f "$zip"
ditto -c -k --keepParent "$app" "$zip"

sha256=$(shasum -a 256 "$zip" | awk '{ print $1 }')
repo=$(github_repo || true)

if [ -f "$cask" ]; then
  "$root/script/update-cask.sh" "$cask" "$version" "$sha256" "$repo"
fi

echo "App     $app"
echo "Zip     $zip"
echo "SHA256  $sha256"
echo "Version $version"
if [ -n "$repo" ]; then
  echo "Cask URL https://github.com/${repo}/releases/download/v${version}/Savage-${version}-macos.zip"
fi
echo
echo "Publish with: git tag v${version} && git push origin v${version}"
echo "Install from this tap after the GitHub release exists:"
echo "  brew tap corkscrews/savage https://github.com/${repo:-Corkscrews/savage}"
echo "  brew install --cask savage"
