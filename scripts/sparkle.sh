#!/usr/bin/env bash
# Fetch the pinned Sparkle release (docs/design/updater.md §5.1).
#
# Usage: scripts/sparkle.sh [--print-dir]
#
# Downloads Sparkle-<version>.tar.xz from Sparkle's GitHub releases,
# checks its SHA-256 against the one below, and unpacks Sparkle.framework
# and bin/sign_update into target/sparkle/<version>/. Nothing is
# committed. A second run with the files already there does nothing.
# Prints the directory (with --print-dir, only that).
#
# SPARKLE_DIR overrides where it goes (default target/sparkle/<version>).
set -euo pipefail

version="2.10.1"
sha256="1a856d2a9edb8e628de88121d7be04cf0e2167f8641650b78fca00b14244cf27"
url="https://github.com/sparkle-project/Sparkle/releases/download/${version}/Sparkle-${version}.tar.xz"

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(dirname "$script_dir")
dir=${SPARKLE_DIR:-"$repo_root/target/sparkle/$version"}

quiet=0
case "${1:-}" in
  --print-dir) quiet=1 ;;
  -h|--help) sed -n '2,12p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
  "") ;;
  *) echo "unknown option $1" >&2; exit 64 ;;
esac
say() { [[ $quiet == 1 ]] || echo "$@" >&2; }

if [[ -f "$dir/.sha256" && "$(cat "$dir/.sha256")" == "$sha256" \
      && -f "$dir/Sparkle.framework/Versions/B/Sparkle" && -x "$dir/bin/sign_update" ]]; then
  echo "$dir"
  exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
archive="$tmp/Sparkle-$version.tar.xz"
say "fetching $url"
curl --fail --location --silent --show-error --retry 3 --output "$archive" "$url"
got=$(shasum -a 256 "$archive" | cut -d' ' -f1)
if [[ "$got" != "$sha256" ]]; then
  echo "Sparkle-$version.tar.xz: SHA-256 is $got, expected $sha256" >&2
  exit 1
fi

# Only the two parts we use; the archive also holds Xcode-only files.
mkdir -p "$tmp/unpacked"
tar -xJf "$archive" -C "$tmp/unpacked" ./Sparkle.framework ./bin/sign_update 2>/dev/null \
  || tar -xJf "$archive" -C "$tmp/unpacked" Sparkle.framework bin/sign_update
rm -rf "$dir"
mkdir -p "$dir/bin"
# ditto keeps the framework's symlinks and signatures as they are.
ditto "$tmp/unpacked/Sparkle.framework" "$dir/Sparkle.framework"
cp "$tmp/unpacked/bin/sign_update" "$dir/bin/sign_update"
chmod 755 "$dir/bin/sign_update"
echo "$sha256" > "$dir/.sha256"
say "Sparkle $version in $dir"
echo "$dir"
