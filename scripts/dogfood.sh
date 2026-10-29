#!/usr/bin/env bash
# Build Terminal Decline and install it as ~/Applications/Terminal Decline.app
# for daily use. A running dogfood build notices the new one and restarts into
# it, reopening its windows in the same folders and places.
#
# The first time, it also updates the pins and removes the build from before
# the rename, ~/Applications/Decline.app; a running copy carries its windows
# over to the new app (see the end of this script).
#
# Usage: scripts/dogfood.sh [--open]
#   --open   start it after installing (otherwise only a running copy restarts)
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
apps="${DECLINE_DOGFOOD_DIR:-$HOME/Applications}"
dest="$apps/Terminal Decline.app"
old_app="$apps/Decline.app"
open_after=0
[[ "${1:-}" == "--open" ]] && open_after=1

cd "$repo"
cargo build --release --locked -p terminal-decline
version="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="terminal-decline"))')-dev+$(git rev-parse --short HEAD)"

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
DOGFOOD=1 scripts/bundle.sh target/release/terminal-decline "$stage" "$version" >/dev/null

mkdir -p "$apps"
# Swap in the new bundle, then remove the old one: removing the old
# executable is what the running copy is waiting for.
if [[ -d "$dest" ]]; then
  mv "$dest" "$dest.old"
fi
mv "$stage/Terminal Decline.app" "$dest"
rm -rf "$dest.old"
lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
"$lsregister" -f "$dest" || true
echo "installed $dest ($version)"

# The build from before the rename, the first time only. Pins made before
# the rename get the new bundle ids (macOS asks for their permissions again).
# Then the old bundle goes: a running copy saves its windows and restarts
# into Terminal Decline.app beside it, which reopens them. A copy too old to
# know about the rename can only fail to restart; it is quit here and the new
# app is opened on the windows it saved.
restore=""
if [[ -d "$old_app" ]]; then
  "$dest/Contents/MacOS/terminal-decline" pin --update || echo "pin --update failed; run it again by hand" >&2
  old_exe="$old_app/Contents/MacOS/decline"
  old_restore="$HOME/Library/Caches/decline/restore.json"
  "$lsregister" -u "$old_app" || true
  if pgrep -f "$old_exe" >/dev/null; then
    rm -f "$old_restore"
    rm -rf "$old_app"
    for _ in $(seq 50); do
      pgrep -f "$old_exe" >/dev/null || break
      sleep 0.1
    done
    if pgrep -f "$old_exe" >/dev/null; then
      pkill -TERM -f "$old_exe" || true
      if [[ -s "$old_restore" ]]; then
        restore="$old_restore"
      else
        echo "the old Decline.app didn't save its windows; opening a new one" >&2
      fi
      open_after=1
    fi
  else
    rm -rf "$old_app"
  fi
  echo "removed $old_app"
fi

if [[ $open_after == 1 ]] && ! pgrep -f "$dest/Contents/MacOS/terminal-decline" >/dev/null; then
  if [[ -n "$restore" ]]; then
    open "$dest" --args --restore "$restore"
  else
    open "$dest"
  fi
fi
