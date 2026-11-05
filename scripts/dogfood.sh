#!/usr/bin/env bash
# Build Terminal Decline and install it as ~/Applications/Terminal Decline.app
# for daily use. A running dogfood build notices the new one and restarts into
# it, reopening its windows in the same folders and places, but never under a
# running program: it waits until every tab is at its prompt and nobody has
# used it for a couple of minutes, or for Update Now (Cmd+U) in the app menu
# (crates/term-restore/src/reload.rs). With --now it restarts at once, for when
# nobody is using it.
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
# The app runs this itself to update (Update Now with new commits waiting),
# with DECLINE_DOGFOOD_DIR set to the folder it is installed in.
#
# Usage: scripts/dogfood.sh [--open] [--now]
#   --open   start it after installing (otherwise only a running copy restarts)
#   --now    a running copy restarts into it at once, ending what runs in it
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
apps="${DECLINE_DOGFOOD_DIR:-$HOME/Applications}"
dest="$apps/Terminal Decline.app"
old_app="$apps/Decline.app"
open_after=0
now=0
for arg in "$@"; do
  case "$arg" in
    --open) open_after=1 ;;
    --now) now=1 ;;
    *) echo "usage: scripts/dogfood.sh [--open] [--now]" >&2; exit 2 ;;
  esac
done
# Read by the running copy when it sees the new build (reload::RESTART_NOW),
# and only if just written: restart now, whatever runs.
restart_now="$HOME/Library/Caches/terminal-decline/restart-now"

cd "$repo"
# The library, then the launcher and stub in a cargo run of their own (their
# profile aborts on panic; the workspace Cargo.toml says why).
cargo build --release --locked -p term-dylib
cargo build --locked -p term-pin --bins --profile release-stub
version="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="terminal-decline"))')-dev+$(git rev-parse --short HEAD)"

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
head=$(git rev-parse HEAD)

# What's New (docs/design/updater.md §12): the changes from the build the
# app last launched (its record in updates-seen, else the installed
# bundle's marker) to this one, as git-cliff groups them, shown once
# after the restart. A start that isn't behind HEAD (another branch, a
# rebuild) gives none. Without git-cliff nothing goes in and the app
# groups the checkout's commits itself, by the same rules.
commit_in() { sed -n 's/^commit=//p' "$1" 2>/dev/null | head -1 || true; }
from=""
for c in "$(commit_in "$HOME/Library/Caches/terminal-decline/updates-seen")" "$(commit_in "$dest/Contents/Resources/dogfood")"; do
  if [[ -n "$c" && "$c" != "$head" ]] && git merge-base --is-ancestor "$c" "$head" 2>/dev/null; then
    from=$c
    break
  fi
done
cliff=$(command -v git-cliff || true)
if [[ -z "$cliff" && -x /opt/homebrew/bin/git-cliff ]]; then
  cliff=/opt/homebrew/bin/git-cliff
fi
whats_new=""
if [[ -n "$from" && -n "$cliff" ]]; then
  whats_new="$stage/whats-new.md"
  if ! { echo "<!-- whats-new from=$from to=$head -->"; "$cliff" --strip header "$from..$head" 2>/dev/null; } > "$whats_new"; then
    whats_new=""
  fi
fi

# The checkout and commit go into the bundle's dogfood marker, so the app
# can check origin/main for new commits and update itself from here
# (docs/design/updater.md §11).
DOGFOOD=1 DOGFOOD_REPO="$repo" DOGFOOD_COMMIT="$head" WHATS_NEW="$whats_new" CODESIGN_IDENTITY="${DECLINE_DOGFOOD_IDENTITY:--}" scripts/bundle.sh "$stage" "$version" >/dev/null

mkdir -p "$apps"
if [[ $now == 1 ]]; then
  mkdir -p "$(dirname "$restart_now")"
  touch "$restart_now"
else
  rm -f "$restart_now"
fi
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
if [[ -n "$from" ]]; then
  if [[ -n "$whats_new" ]]; then
    changes=$(grep -c '^- ' "$whats_new" || true)
    echo "what's new: $changes change(s) since ${from:0:9}, shown once after the restart"
  else
    echo "what's new: since ${from:0:9} (git-cliff not found: the app lists the commits itself)"
  fi
fi
if pgrep -f "$dest/Contents/MacOS/terminal-decline" >/dev/null; then
  if [[ $now == 1 ]]; then
    echo "installed; the app restarts into it now"
  else
    echo "installed; the app restarts when you're idle, or press Cmd+U (Update Now)"
  fi
fi

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
