#!/bin/sh
set -eu

cask=$1
version=$2
sha256=$3
repo=${4:-}

if [ -z "$cask" ] || [ -z "$version" ] || [ -z "$sha256" ]; then
  echo "usage: update-cask.sh <Casks/savage.rb> <version> <sha256> [owner/repo]" >&2
  exit 1
fi

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT

awk -v ver="$version" -v sha="$sha256" '
  /^  version / { print "  version \"" ver "\""; next }
  /^  sha256 /  { print "  sha256 \"" sha "\""; next }
  { print }
' "$cask" > "$tmp"

if [ -n "$repo" ]; then
  escaped=$(printf '%s' "$repo" | sed 's/[\/&]/\\&/g')
  sed "s#github.com/[A-Za-z0-9_.-]*/[A-Za-z0-9_.-]*#github.com/${escaped}#g" "$tmp" > "$cask"
else
  cp "$tmp" "$cask"
fi
