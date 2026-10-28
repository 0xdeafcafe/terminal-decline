#!/usr/bin/env bash
# Build Decline.app from a compiled binary and ad-hoc codesign it.
# Usage: scripts/bundle.sh <binary> <output-dir> [version] [build-number]
# The build number must be an integer that only goes up (Sparkle and
# LaunchServices compare it); it defaults to the commit count.
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "usage: $0 <binary> <output-dir> [version] [build-number]" >&2
  exit 64
fi

binary=$1
out_dir=$2
version=${3:-0.0.0}
build=${4:-$(git -C "$(dirname "$0")" rev-list --count HEAD 2>/dev/null || echo 1)}
if ! [[ "$build" =~ ^[0-9]+$ ]]; then
  echo "build number must be an integer: $build" >&2
  exit 64
fi

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(dirname "$script_dir")
icon="$repo_root/packaging/icon.icns"

if [[ ! -f "$binary" ]]; then
  echo "binary not found: $binary" >&2
  exit 66
fi

app="$out_dir/Decline.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/decline"
chmod 755 "$app/Contents/MacOS/decline"

icon_entry=""
if [[ -f "$icon" ]]; then
  cp "$icon" "$app/Contents/Resources/icon.icns"
  icon_entry="  <key>CFBundleIconFile</key>
  <string>icon</string>"
fi

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>red.forbes.decline</string>
  <key>CFBundleName</key>
  <string>Decline</string>
  <key>CFBundleDisplayName</key>
  <string>Decline</string>
  <key>CFBundleExecutable</key>
  <string>decline</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${version}</string>
  <key>CFBundleVersion</key>
  <string>${build}</string>
  <key>LSMinimumSystemVersion</key>
  <string>14.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
${icon_entry}
</dict>
</plist>
EOF

# CODESIGN_IDENTITY defaults to ad-hoc ("-"); set it for real Developer ID signing.
identity=${CODESIGN_IDENTITY:--}
sign_args=(--force --deep --sign "$identity")
if [[ "$identity" != "-" ]]; then
  sign_args+=(--options runtime --timestamp)
fi
codesign "${sign_args[@]}" "$app"
codesign --verify --deep --strict "$app"

echo "$app"
