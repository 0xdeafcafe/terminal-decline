#!/usr/bin/env bash
# Build Terminal Decline and install it as ~/Applications/Terminal Decline.app
# for daily use. A running dogfood build notices the new one and restarts into
# it, reopening its windows in the same folders and places.
#
# The bundle is the shipping layout (docs/design/pins.md §2): the launcher,
# libterminal_decline.dylib and the pin stub. It also writes
# ~/.config/terminal-decline/pin-app, so pins made from stubs load this
# build's library (pins.md §4.3).
#
# The first time, it also removes the build from before the rename,
# ~/Applications/Decline.app; a running copy carries its windows over to the
# new app (see the end of this script).
#
# Signed ad-hoc unless DECLINE_DOGFOOD_IDENTITY names a codesigning identity
# (a Developer ID Application certificate in the keychain; pins.md §11,
# question 2), which signs with the hardened runtime and a timestamp.
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
# The library, then the launcher and stub in a cargo run of their own (their
# profile aborts on panic; the workspace Cargo.toml says why).
cargo build --release --locked -p term-dylib
cargo build --locked -p term-pin --bins --profile release-stub
version="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="terminal-decline"))')-dev+$(git rev-parse --short HEAD)"

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
DOGFOOD=1 CODESIGN_IDENTITY="${DECLINE_DOGFOOD_IDENTITY:--}" scripts/bundle.sh "$stage" "$version" >/dev/null

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

# Where pins look first for the app (crates/term-pin/FROZEN.md): always
# $HOME/.config, whatever XDG_CONFIG_HOME says, as a Dock launch has none.
mkdir -p "$HOME/.config/terminal-decline"
printf '%s\n' "$(cd "$apps" && pwd -P)/Terminal Decline.app" > "$HOME/.config/terminal-decline/pin-app"

# Pins (pins.md §5.3): registered again, followed if moved, and clone pins
# from older builds (before or after the rename) rebuilt as stub pins. Each
# rebuild resets that pin's permissions, so it asks first; without a
# terminal the answer is no. Stub pins are never rewritten.
"$dest/Contents/MacOS/terminal-decline" pin --repair || echo "pin --repair failed; run it again by hand" >&2

# The build from before the rename, the first time only. Its pins were
# rebuilt with the current ids by `pin --repair` above, if you said yes.
# The old bundle goes: a running copy saves its windows and restarts
# into Terminal Decline.app beside it, which reopens them. A copy too old to
# know about the rename can only fail to restart; it is quit here and the new
# app is opened on the windows it saved.
restore=""
if [[ -d "$old_app" ]]; then
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

# Read the Services menu items (Info.plist NSServices) again.
/System/Library/CoreServices/pbs -update || true

if [[ $open_after == 1 ]] && ! pgrep -f "$dest/Contents/MacOS/terminal-decline" >/dev/null; then
  if [[ -n "$restore" ]]; then
    open "$dest" --args --restore "$restore"
  else
    open "$dest"
  fi
fi
