#!/usr/bin/env bash
# Build Terminal Decline.app from a compiled binary and ad-hoc codesign it.
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

# The bundle's name (the .app and CFBundleName) and the name people see
# in menus. Set here only.
bundle_name="Terminal Decline"
product_name="Terminal Decline"

app="$out_dir/$bundle_name.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/terminal-decline"
chmod 755 "$app/Contents/MacOS/terminal-decline"

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
  <string>red.forbes.terminal-decline</string>
  <key>CFBundleName</key>
  <string>${bundle_name}</string>
  <key>CFBundleDisplayName</key>
  <string>${bundle_name}</string>
  <key>CFBundleExecutable</key>
  <string>terminal-decline</string>
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
  <!-- Folders can be dropped on the Dock icon: each opens a window there
       (application:openURLs:). -->
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key>
      <string>Folder</string>
      <key>CFBundleTypeRole</key>
      <string>Shell</string>
      <key>LSHandlerRank</key>
      <string>None</string>
      <key>LSItemContentTypes</key>
      <array>
        <string>public.folder</string>
      </array>
    </dict>
  </array>
  <!-- Finder's Services menu (and the right-click menu's Services) for a
       folder: the selectors are in crates/term-app/src/app.rs. -->
  <key>NSServices</key>
  <array>
    <dict>
      <key>NSMenuItem</key>
      <dict>
        <key>default</key>
        <string>New ${product_name} Window at Folder</string>
      </dict>
      <key>NSMessage</key>
      <string>newWindowAtFolder</string>
      <key>NSPortName</key>
      <string>${bundle_name}</string>
      <key>NSRequiredContext</key>
      <dict/>
      <key>NSSendFileTypes</key>
      <array>
        <string>public.folder</string>
      </array>
    </dict>
    <dict>
      <key>NSMenuItem</key>
      <dict>
        <key>default</key>
        <string>New ${product_name} Tab at Folder</string>
      </dict>
      <key>NSMessage</key>
      <string>newTabAtFolder</string>
      <key>NSPortName</key>
      <string>${bundle_name}</string>
      <key>NSRequiredContext</key>
      <dict/>
      <key>NSSendFileTypes</key>
      <array>
        <string>public.folder</string>
      </array>
    </dict>
  </array>
${icon_entry}
</dict>
</plist>
EOF

# DOGFOOD=1 marks a local build that restarts itself when a newer one is
# installed (see crates/term-app/src/reload.rs and scripts/dogfood.sh).
if [[ "${DOGFOOD:-}" == "1" ]]; then
  touch "$app/Contents/Resources/dogfood"
fi

# CODESIGN_IDENTITY defaults to ad-hoc ("-"); set it for real Developer ID signing.
identity=${CODESIGN_IDENTITY:--}
sign_args=(--force --deep --sign "$identity")
if [[ "$identity" != "-" ]]; then
  sign_args+=(--options runtime --timestamp)
fi
codesign "${sign_args[@]}" "$app"
codesign --verify --deep --strict "$app"

echo "$app"
