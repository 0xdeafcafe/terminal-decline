#!/usr/bin/env bash
# Build Terminal Decline.app from compiled parts and sign it, inside out
# (docs/design/pins.md §2.1, §2.4).
#
# Usage: scripts/bundle.sh [--lib FILE] [--launcher FILE] [--stub FILE]
#                          <output-dir> [version] [build-number]
#
#   --lib       libterminal_decline.dylib (crates/term-dylib; default
#               target/release/libterminal_decline.dylib)
#   --launcher  the launcher (crates/term-pin; default
#               target/release-stub/terminal-decline-launcher)
#   --stub      the pin stub (crates/term-pin; default
#               target/release-stub/terminal-decline-pin)
#
# Build them with:
#   cargo build --release -p term-dylib
#   cargo build -p term-pin --bins --profile release-stub
#
# The build number must be an integer that only goes up (Sparkle and
# LaunchServices compare it); it defaults to the commit count.
#
# Signing: CODESIGN_IDENTITY (default "-", ad-hoc). A real identity signs
# with the hardened runtime and a timestamp, and packaging/entitlements.plist.
# Ad-hoc builds are hardened only with HARDENED=1, then with
# packaging/entitlements-dev.plist (library validation off: ad-hoc code
# has no team ID to match). DOGFOOD=1 marks a local build that restarts
# itself when a newer one is installed (crates/term-app/src/reload.rs);
# with DOGFOOD_REPO and DOGFOOD_COMMIT, its marker names the checkout and
# commit, for updates from source (crates/term-app/src/update/source.rs).
#
# Sparkle (docs/design/updater.md §5): when scripts/sparkle.sh has fetched
# it (target/sparkle/<version>, or SPARKLE_DIR), Sparkle.framework is
# embedded, thinned and signed, and the updater's Info.plist keys are
# written. SPARKLE_REQUIRED=1 (the release job) fails without it. The
# updater still only runs in Developer ID builds that aren't dogfood or
# nightly builds (crates/term-app/src/update.rs).
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(dirname "$script_dir")

lib="$repo_root/target/release/libterminal_decline.dylib"
launcher="$repo_root/target/release-stub/terminal-decline-launcher"
stub="$repo_root/target/release-stub/terminal-decline-pin"
positional=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --lib) lib=$2; shift 2 ;;
    --launcher) launcher=$2; shift 2 ;;
    --stub) stub=$2; shift 2 ;;
    -h|--help)
      sed -n '2,34p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    -*) echo "unknown option $1" >&2; exit 64 ;;
    *) positional+=("$1"); shift ;;
  esac
done
if [[ ${#positional[@]} -lt 1 ]]; then
  echo "usage: $0 [--lib FILE] [--launcher FILE] [--stub FILE] <output-dir> [version] [build-number]" >&2
  exit 64
fi

out_dir=${positional[0]}
version=${positional[1]:-0.0.0}
build=${positional[2]:-$(git -C "$script_dir" rev-list --count HEAD 2>/dev/null || echo 1)}
if ! [[ "$build" =~ ^[0-9]+$ ]]; then
  echo "build number must be an integer: $build" >&2
  exit 64
fi

for part in "$lib" "$launcher" "$stub"; do
  if [[ ! -f "$part" ]]; then
    echo "not found: $part (see $0 --help)" >&2
    exit 66
  fi
done

icon="$repo_root/packaging/icon.icns"

# The bundle's name (the .app and CFBundleName) and the name people see
# in menus. Set here only.
bundle_name="Terminal Decline"
product_name="Terminal Decline"
bundle_id="red.forbes.terminal-decline"

app="$out_dir/$bundle_name.app"
contents="$app/Contents"
rm -rf "$app"
mkdir -p "$contents/MacOS" "$contents/Frameworks" "$contents/Helpers" "$contents/Resources"
# The launcher is named terminal-decline in the bundle (the cask links it
# as `terminal-decline` and `decline`).
cp "$launcher" "$contents/MacOS/terminal-decline"
cp "$lib" "$contents/Frameworks/libterminal_decline.dylib"
cp "$stub" "$contents/Helpers/terminal-decline-pin"
chmod 755 "$contents/MacOS/terminal-decline" "$contents/Helpers/terminal-decline-pin"
chmod 644 "$contents/Frameworks/libterminal_decline.dylib"

# Sparkle.framework, without what only an Xcode build or a sandboxed app
# uses: the XPC services (the folder and its top-level symlink, or
# `codesign --verify --strict` fails on the dangling link), headers,
# modules and other localisations. Thinned to the library's architecture.
sparkle_version=$(sed -n 's/^version="\(.*\)"$/\1/p' "$script_dir/sparkle.sh")
sparkle_dir=${SPARKLE_DIR:-"$repo_root/target/sparkle/$sparkle_version"}
fw="$contents/Frameworks/Sparkle.framework"
sparkle=0
if [[ -f "$sparkle_dir/Sparkle.framework/Versions/B/Sparkle" ]]; then
  sparkle=1
  ditto "$sparkle_dir/Sparkle.framework" "$fw"
  rm -rf "$fw/Versions/B/XPCServices" "$fw/XPCServices" \
    "$fw/Versions/B/Headers" "$fw/Headers" \
    "$fw/Versions/B/PrivateHeaders" "$fw/PrivateHeaders" \
    "$fw/Versions/B/Modules" "$fw/Modules" \
    "$fw/Versions/B/_CodeSignature"
  find "$fw/Versions/B/Resources" -maxdepth 1 -name '*.lproj' ! -name 'en.lproj' ! -name 'Base.lproj' -exec rm -rf {} +
  # Sparkle's licence (and those of the code it bundles) goes with it;
  # a new Sparkle version needs its licence checked there too.
  if ! grep -qx "## Sparkle $sparkle_version" "$repo_root/THIRD-PARTY-NOTICES.md"; then
    echo "THIRD-PARTY-NOTICES.md has no section for Sparkle $sparkle_version" >&2
    exit 1
  fi
  cp "$repo_root/THIRD-PARTY-NOTICES.md" "$contents/Resources/THIRD-PARTY-NOTICES.md"
  arch=$(lipo -archs "$lib" 2>/dev/null || true)
  if [[ "$arch" == "arm64" || "$arch" == "x86_64" ]]; then
    for bin in "$fw/Versions/B/Sparkle" "$fw/Versions/B/Autoupdate" "$fw/Versions/B/Updater.app/Contents/MacOS/Updater"; do
      if [[ "$(lipo -archs "$bin")" != "$arch" ]]; then
        lipo -thin "$arch" "$bin" -output "$bin.thin"
        mv "$bin.thin" "$bin"
      fi
    done
  fi
elif [[ "${SPARKLE_REQUIRED:-}" == "1" ]]; then
  echo "no Sparkle.framework in $sparkle_dir (run scripts/sparkle.sh)" >&2
  exit 66
fi

# The updater's keys (docs/design/updater.md §5.2). Without a public key
# the updater stays off.
sparkle_keys=""
if [[ $sparkle == 1 ]]; then
  public_key=""
  if [[ -f "$repo_root/packaging/sparkle-public-key" ]]; then
    public_key=$(tr -d '[:space:]' < "$repo_root/packaging/sparkle-public-key")
  fi
  if [[ -n "$public_key" ]]; then
    sparkle_keys="  <key>SUPublicEDKey</key>
  <string>${public_key}</string>
"
  fi
  sparkle_keys+="  <key>SUFeedURL</key>
  <string>https://github.com/0xdeafcafe/terminal-decline/releases/latest/download/appcast.xml</string>
  <key>SUEnableAutomaticChecks</key>
  <true/>
  <key>SUScheduledCheckInterval</key>
  <integer>86400</integer>
  <key>SUAutomaticallyUpdate</key>
  <true/>
  <key>SUAllowsAutomaticUpdates</key>
  <true/>
  <key>SUEnableSystemProfiling</key>
  <false/>
  <key>SUVerifyUpdateBeforeExtraction</key>
  <true/>
  <key>SURequireSignedFeed</key>
  <true/>
  <!-- An update that has waited a week asks to Quit and Install. -->
  <key>SUScheduledImpatientCheckInterval</key>
  <integer>604800</integer>
"
fi

icon_entry=""
if [[ -f "$icon" ]]; then
  cp "$icon" "$contents/Resources/icon.icns"
  icon_entry="  <key>CFBundleIconFile</key>
  <string>icon</string>"
fi

# Programs run in a terminal ask macOS for access as the terminal, so the
# app declares every kind of access one might ask for (pins.md §2.3).
usage_strings=""
for entry in \
  "NSMicrophoneUsageDescription:use the microphone" \
  "NSCameraUsageDescription:use the camera" \
  "NSAudioCaptureUsageDescription:record audio from other apps" \
  "NSAppleEventsUsageDescription:control other apps" \
  "NSAppleMusicUsageDescription:use your music library" \
  "NSLocationUsageDescription:use your location" \
  "NSLocationWhenInUseUsageDescription:use your location" \
  "NSContactsUsageDescription:use your contacts" \
  "NSCalendarsUsageDescription:use your calendars" \
  "NSCalendarsFullAccessUsageDescription:use your calendars" \
  "NSRemindersUsageDescription:use your reminders" \
  "NSRemindersFullAccessUsageDescription:use your reminders" \
  "NSPhotoLibraryUsageDescription:use your photos" \
  "NSBluetoothAlwaysUsageDescription:use Bluetooth" \
  "NSLocalNetworkUsageDescription:find and connect to devices on your local network" \
  "NSSpeechRecognitionUsageDescription:use speech recognition" \
  "NSDesktopFolderUsageDescription:use files in your Desktop folder" \
  "NSDocumentsFolderUsageDescription:use files in your Documents folder" \
  "NSDownloadsFolderUsageDescription:use files in your Downloads folder" \
  "NSRemovableVolumesUsageDescription:use files on a removable volume" \
  "NSNetworkVolumesUsageDescription:use files on a network volume" \
  "NSSystemAdministrationUsageDescription:change system settings"; do
  usage_strings+="  <key>${entry%%:*}</key>
  <string>A program running in ${product_name} wants to ${entry#*:}.</string>
"
done

cat > "$contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>${bundle_id}</string>
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
${usage_strings}${sparkle_keys}${icon_entry}
</dict>
</plist>
EOF

if [[ "${DOGFOOD:-}" == "1" ]]; then
  # Where it was built from (dogfood.sh sets these), before signing seals
  # the bundle: the app checks that checkout for updates.
  if [[ -n "${DOGFOOD_REPO:-}" && -n "${DOGFOOD_COMMIT:-}" ]]; then
    printf 'repo=%s\ncommit=%s\n' "$DOGFOOD_REPO" "$DOGFOOD_COMMIT" > "$contents/Resources/dogfood"
  else
    touch "$contents/Resources/dogfood"
  fi
fi

# Inside out, never --deep (it would sign nested code with the app's
# identifier, entitlements and options): the library, then the stub, then
# the app, which signs the launcher and seals the bundle.
identity=${CODESIGN_IDENTITY:--}
opts=()
entitlements="$repo_root/packaging/entitlements.plist"
if [[ "$identity" != "-" ]]; then
  opts=(--options runtime --timestamp)
elif [[ "${HARDENED:-}" == "1" ]]; then
  opts=(--options runtime)
  entitlements="$repo_root/packaging/entitlements-dev.plist"
fi
sign() {
  codesign --force --sign "$identity" ${opts[@]+"${opts[@]}"} "$@"
}
sign --identifier "$bundle_id.lib" "$contents/Frameworks/libterminal_decline.dylib"
# Sparkle's helpers, then Sparkle.framework (docs/design/updater.md §5.3).
if [[ $sparkle == 1 ]]; then
  sign "$fw/Versions/B/Autoupdate"
  sign "$fw/Versions/B/Updater.app"
  sign "$fw"
fi
sign --identifier "$bundle_id.pin-stub" "$contents/Helpers/terminal-decline-pin"
sign --entitlements "$entitlements" "$app"
codesign --verify --strict --deep "$app"

echo "$app"
