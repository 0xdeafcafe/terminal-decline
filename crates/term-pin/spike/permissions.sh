#!/bin/bash
# P-0's permission check, the part that needs a person to click
# (docs/design/pins.md §12): does a grant given to a pin survive swapping
# the library it loads?
#
#   crates/term-pin/spike/permissions.sh
#
# Makes a pin "TD Probe" (red.forbes.terminal-decline.pin.tdprobe) in a temp
# dir, with a fake Terminal Decline.app whose library is the C fixture
# built with -DPROBE: it reports whether ~/Documents, ~/Library/Mail and
# ~/Library/Safari can be read. The pin is started with `open`, so macOS
# treats it as its own app (started from a terminal, it would be the
# terminal asking). Steps:
#
#   1. Run A: macOS asks "TD Probe would like to access files in your
#      Documents folder". Allow.
#   2. The library is replaced by build B (another Developer ID
#      signature). Run B: expected no question, Documents readable.
#   3. You add TD Probe to System Settings > Privacy & Security > Full Disk
#      Access (the script opens the pane and shows the pin in Finder; drag
#      it in). Run C: Mail/Safari readable.
#   4. The library is replaced by build A again. Run D: expected Mail and
#      Safari still readable, no question.
#
# At the end the grants are removed (tccutil reset) and the pin is
# unregistered. Nothing goes into ~/Applications.
set -euo pipefail

root=$(cd "$(dirname "$0")/../../.." && pwd)
S=$(mktemp -d)
S=$(cd "$S" && pwd -P)
ID=${IDENTITY:-$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: .*\)"/\1/p' | head -1)}
[ -n "$ID" ] || { echo "no Developer ID identity in the keychain" >&2; exit 1; }
TEAM=$(printf '%s' "$ID" | sed -n 's/.*(\([A-Z0-9]*\))$/\1/p')
LIBID=red.forbes.terminal-decline.lib
APPID=red.forbes.terminal-decline.spike
PINID=red.forbes.terminal-decline.pin.tdprobe
REQ="anchor apple generic and certificate leaf[subject.OU] = \"$TEAM\" and identifier \"$LIBID\""

cargo build -q --release -p term-pin --manifest-path "$root/Cargo.toml"
for b in A B; do
    xcrun clang -dynamiclib -O2 -DPROBE -DBUILD="\"$b\"" -o "$S/probe-$b.dylib" "$root/crates/term-pin/tests/fixture/fixture.c"
    codesign -f -s "$ID" -o runtime --timestamp -i "$LIBID" "$S/probe-$b.dylib" 2>/dev/null
done

APP="$S/Terminal Decline.app"
mkdir -p "$APP/Contents/Frameworks"
plutil -create xml1 "$APP/Contents/Info.plist"
plutil -insert CFBundleIdentifier -string "$APPID" "$APP/Contents/Info.plist"
plutil -insert CFBundleVersion -string 1 "$APP/Contents/Info.plist"
H="$S/home"
mkdir -p "$H/.config/terminal-decline"
echo "$APP" >"$H/.config/terminal-decline/pin-app"

PIN="$S/TD Probe.app"
mkdir -p "$PIN/Contents/MacOS"
cp "$root/target/release/terminal-decline-pin" "$PIN/Contents/MacOS/tdprobe"
P="$PIN/Contents/Info.plist"
plutil -create xml1 "$P"
plutil -insert CFBundleIdentifier -string "$PINID" "$P"
plutil -insert CFBundleExecutable -string tdprobe "$P"
plutil -insert CFBundleName -string "TD Probe" "$P"
plutil -insert CFBundlePackageType -string APPL "$P"
plutil -insert CFBundleShortVersionString -string 1 "$P"
plutil -insert CFBundleVersion -string 1 "$P"
plutil -insert NSDocumentsFolderUsageDescription -string "A program running in TD Probe (a Terminal Decline pin) wants to use your Documents folder." "$P"
plutil -insert TDPinAbi -integer 1 "$P"
plutil -insert TDPinId -string tdprobe "$P"
plutil -insert TDAppBundleId -string "$APPID" "$P"
plutil -insert TDLibraryRequirement -string "$REQ" "$P"
E="$S/ents.plist"
plutil -create xml1 "$E"
plutil -insert com\\.apple\\.security\\.cs\\.disable-library-validation -bool true "$E"
codesign -f -s - -o runtime --entitlements "$E" -i "$PINID" "$PIN" 2>/dev/null
hash0=$(codesign -dvvv "$PIN" 2>&1 | sed -n 's/^CDHash=//p')
echo "pin: $PIN (CDHash $hash0)"
if [ "${DRY:-}" = 1 ]; then # check the setup without starting the pin
    codesign --verify --strict "$PIN" && codesign -v -R="$REQ" "$S/probe-A.dylib" && codesign -v -R="$REQ" "$S/probe-B.dylib" && echo "setup ok"
    rm -rf "$S"
    exit 0
fi

run() { # run LABEL BUILD
    cp -f "$S/probe-$2.dylib" "$APP/Contents/Frameworks/libterminal_decline.dylib"
    rm -f "$S/out-$1"
    open -n -W --env HOME="$H" "$PIN" --args "$S/out-$1"
    echo "-- run $1 (library build $2), pin CDHash $(codesign -dvvv "$PIN" 2>&1 | sed -n 's/^CDHash=//p')"
    grep -E '^(build|Documents|Library/Mail|Library/Safari)=' "$S/out-$1" || echo "   no output: the pin didn't start"
}

read -r -p "Step 1: run A. macOS should ask about Documents: click Allow. [Return] "
run 1 A
read -r -p "Step 2: run B with the other library. Expected: no question. [Return] "
run 2 B
echo "Step 3: add TD Probe to Full Disk Access (drag it from the Finder window into the list), then come back."
open "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_AllFiles"
open -R "$PIN"
read -r -p "[Return when added] "
run 3 B
read -r -p "Step 4: run D with library A again. Expected: no question, Mail/Safari readable. [Return] "
run 4 A

echo "-- cleaning up"
tccutil reset SystemPolicyDocumentsFolder "$PINID" || true
tccutil reset SystemPolicyAllFiles "$PINID" || true
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -u "$PIN" || true
rm -rf "$S"
echo "done; paste the four results into docs/design/pins.md §12"
