#!/usr/bin/env bash
# Build decline and install it as ~/Applications/Decline.app for daily use.
# A running dogfood build notices the new one and restarts into it, reopening
# its windows in the same folders and places.
#
# Usage: scripts/dogfood.sh [--open]
#   --open   start it after installing (otherwise only a running copy restarts)
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
dest="${DECLINE_DOGFOOD_DIR:-$HOME/Applications}/Decline.app"
open_after=0
[[ "${1:-}" == "--open" ]] && open_after=1

cd "$repo"
cargo build --release --locked -p decline
version="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="decline"))')-dev+$(git rev-parse --short HEAD)"

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
DOGFOOD=1 scripts/bundle.sh target/release/decline "$stage" "$version" >/dev/null

mkdir -p "$(dirname "$dest")"
# Swap in the new bundle, then remove the old one: removing the old
# executable is what the running copy is waiting for.
if [[ -d "$dest" ]]; then
  mv "$dest" "$dest.old"
fi
mv "$stage/Decline.app" "$dest"
rm -rf "$dest.old"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$dest" || true

echo "installed $dest ($version)"
if [[ $open_after == 1 ]] && ! pgrep -f "$dest/Contents/MacOS/decline" >/dev/null; then
  open "$dest"
fi
