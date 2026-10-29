#!/bin/bash
# P-0's signing spike (docs/design/pins.md §10, results in §0 and §12).
#
#   crates/term-pin/spike/run.sh [SCRATCH_DIR]
#
# Builds the minimal stub, the C fixture library and (with SPIKE_APP=1)
# today's app as a library, makes a fake Terminal Decline.app and pins in
# SCRATCH_DIR (default: a new temp dir), and checks the claims the design
# builds on. Never touches ~/Applications or real pins: every bundle is in
# SCRATCH_DIR and HOME points there. IDENTITY (default: the first
# "Developer ID Application" identity in the keychain) signs the release
# side; without one only the ad-hoc checks run.
set -euo pipefail

root=$(cd "$(dirname "$0")/../../.." && pwd)
S=${1:-$(mktemp -d)}
mkdir -p "$S"
S=$(cd "$S" && pwd -P)
ID=${IDENTITY:-$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: .*\)"/\1/p' | head -1)}
LIBID=red.forbes.terminal-decline.lib
TEAM=$(printf '%s' "$ID" | sed -n 's/.*(\([A-Z0-9]*\))$/\1/p')
echo "scratch: $S"
echo "identity: ${ID:-none (ad-hoc only)}"

# P-0's minimal stub (examples/spike-stub.rs), which can time its steps.
cargo build -q --release -p term-pin --example spike-stub --manifest-path "$root/Cargo.toml"
STUB="$root/target/release/examples/spike-stub"
FIX="$root/crates/term-pin/tests/fixture/fixture.c"

ok() { printf '  ok    %s\n' "$*"; }
bad() { printf '  FAIL  %s\n' "$*"; }

# fixture NAME BUILD SIGN-IDENTITY [extra clang flags]
fixture() {
    local out="$S/lib/$1.dylib"
    mkdir -p "$S/lib"
    xcrun clang -dynamiclib -O2 -DBUILD="\"$2\"" "${@:4}" -o "$out" "$FIX"
    if [ "$3" = - ]; then
        codesign -f -s - -i "$LIBID" "$out" 2>&1 | grep -v "replacing existing" >&2 || true
    else
        codesign -f -s "$3" -o runtime --timestamp -i "$LIBID" "$out" 2>&1 | grep -v "replacing existing" >&2 || true
    fi
    echo "$out"
}

# app DIR LIBRARY: a fake Terminal Decline.app holding LIBRARY
app() {
    mkdir -p "$1/Contents/Frameworks"
    cat >"$1/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>red.forbes.terminal-decline.spike</string>
<key>CFBundleExecutable</key><string>terminal-decline</string>
<key>CFBundleVersion</key><string>1</string>
</dict></plist>
EOF
    cp -f "$2" "$1/Contents/Frameworks/libterminal_decline.dylib"
}

# pin DIR ID REQUIREMENT [no-dlv]: a pin bundle around the stub, signed
# ad-hoc, hardened, with library validation off unless no-dlv
pin() {
    local dir=$1 id=$2 req=$3
    rm -rf "$dir"
    mkdir -p "$dir/Contents/MacOS"
    cp "$STUB" "$dir/Contents/MacOS/$id"
    cat >"$dir/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>red.forbes.terminal-decline.pin.$id</string>
<key>CFBundleExecutable</key><string>$id</string>
<key>CFBundleName</key><string>$id</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>1</string>
<key>CFBundleVersion</key><string>1</string>
<key>NSDocumentsFolderUsageDescription</key><string>A program running in $id (a Terminal Decline pin) wants to use your Documents folder.</string>
<key>TDPinAbi</key><integer>1</integer>
<key>TDPinId</key><string>$id</string>
<key>TDAppBundleId</key><string>red.forbes.terminal-decline.spike</string>
<key>TDLibraryRequirement</key><string>$req</string>
</dict></plist>
EOF
    local ents="$S/ents.plist"
    if [ "${4:-}" = no-dlv ]; then
        ents="$S/ents-no-dlv.plist"
    fi
    codesign -f -s - -o runtime --entitlements "$ents" -i "red.forbes.terminal-decline.pin.$id" "$dir" 2>/dev/null
}

cat >"$S/ents.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>com.apple.security.cs.disable-library-validation</key><true/>
<key>com.apple.security.device.audio-input</key><true/>
<key>com.apple.security.device.camera</key><true/>
<key>com.apple.security.automation.apple-events</key><true/>
<key>com.apple.security.personal-information.location</key><true/>
<key>com.apple.security.personal-information.addressbook</key><true/>
<key>com.apple.security.personal-information.calendars</key><true/>
<key>com.apple.security.personal-information.photos-library</key><true/>
</dict></plist>
EOF
sed '/disable-library-validation/d' "$S/ents.plist" >"$S/ents-no-dlv.plist"

H="$S/home"
mkdir -p "$H/.config/terminal-decline"
# HOME only for the stub: codesign finds the keychain through it.
cdhash() { codesign -dvvv "$1" 2>&1 | sed -n 's/^CDHash=//p'; }
run_pin() { # run_pin PIN-DIR ID OUT [env...]
    local p=$1 id=$2 out=$3
    shift 3
    rm -f "$out"
    env HOME="$H" "$@" "$p/Contents/MacOS/$id" "$out"
}

DEV_REQ="identifier \"$LIBID\""
echo "== ad-hoc (dev pin)"
A=$(fixture adhoc-A A -)
app "$S/apps/TD.app" "$A"
echo "$S/apps/TD.app" >"$H/.config/terminal-decline/pin-app"
pin "$S/pins/dev.app" dev "$DEV_REQ"
set +e
run_pin "$S/pins/dev.app" dev "$S/out"
rc=$?
set -e
grep -q build=A "$S/out" && [ $rc = 7 ] && ok "dev pin runs an ad-hoc library (exit $rc)" || bad "dev pin: exit $rc"

if [ -n "$ID" ]; then
    TEAM_REQ="anchor apple generic and certificate leaf[subject.OU] = \"$TEAM\" and identifier \"$LIBID\""
    DEVID_REQ="anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = \"$TEAM\" and identifier \"$LIBID\""
    echo "== Developer ID ($TEAM)"
    RA=$(fixture devid-A A "$ID")
    RB=$(fixture devid-B B "$ID")
    codesign -v -R="$TEAM_REQ" "$RA" && ok "codesign: the team requirement passes" || bad "codesign: team requirement"
    codesign -v -R="$DEVID_REQ" "$RA" && ok "codesign: team + Developer ID marker passes" || bad "codesign: Developer ID marker"

    app "$S/apps/TD.app" "$RA"
    pin "$S/pins/rel.app" rel "$TEAM_REQ"
    set +e
    run_pin "$S/pins/rel.app" rel "$S/out"
    rc=$?
    set -e
    grep -q build=A "$S/out" && ok "release pin (ad-hoc, hardened, DLV) loads the Developer ID library" || bad "release pin: exit $rc"

    pin "$S/pins/reldv.app" reldv "$DEVID_REQ"
    set +e
    run_pin "$S/pins/reldv.app" reldv "$S/out"
    set -e
    grep -q build=A "$S/out" && ok "the stub's check accepts the Developer ID marker requirement" || bad "Developer ID marker in the stub"

    app "$S/apps/TD.app" "$A"
    set +e
    msg=$(run_pin "$S/pins/rel.app" rel "$S/out" 2>&1)
    rc=$?
    set -e
    [ $rc = 1 ] && [ ! -e "$S/out" ] && ok "release pin refuses the ad-hoc library: $msg" || bad "release pin with ad-hoc library: exit $rc"

    app "$S/apps/TD.app" "$RA"
    pin "$S/pins/nodlv.app" nodlv "$TEAM_REQ" no-dlv
    set +e
    msg=$(run_pin "$S/pins/nodlv.app" nodlv "$S/out" 2>&1)
    rc=$?
    set -e
    [ $rc != 7 ] && ok "without disable-library-validation the Developer ID library is refused: ${msg##*: }" || bad "no-DLV pin loaded it"

    echo "== identity across a library swap"
    before=$(cdhash "$S/pins/rel.app")
    dr_before=$(codesign -d -r- "$S/pins/rel.app" 2>&1 | sed -n 's/^# designated => //p')
    app "$S/apps/TD.app" "$RB"
    set +e
    run_pin "$S/pins/rel.app" rel "$S/out"
    set -e
    after=$(cdhash "$S/pins/rel.app")
    dr_after=$(codesign -d -r- "$S/pins/rel.app" 2>&1 | sed -n 's/^# designated => //p')
    grep -q build=B "$S/out" && [ "$before" = "$after" ] && [ "$dr_before" = "$dr_after" ] &&
        ok "library A -> B: B ran, pin CDHash $after and designated requirement unchanged ($dr_after)" || bad "swap changed the pin"
    codesign --verify --strict "$S/pins/rel.app" && ok "pin still passes codesign --verify --strict"

    echo "== /dev/fd/N"
    set +e
    msg=$(run_pin "$S/pins/rel.app" rel "$S/out" TD_PIN_SPIKE_FD=1 2>&1)
    rc=$?
    set -e
    [ $rc = 7 ] && ok "dlopen(/dev/fd/N) works" || ok "dlopen(/dev/fd/N) refused: ${msg##*: }"

    echo "== DYLD_INSERT_LIBRARIES"
    INJ="$S/lib/inject.dylib"
    xcrun clang -dynamiclib -DINJECT -o "$INJ" "$FIX"
    codesign -f -s - "$INJ" 2>/dev/null
    rm -f "$S/injected"
    set +e
    run_pin "$S/pins/rel.app" rel "$S/out" DYLD_INSERT_LIBRARIES="$INJ" TD_FIXTURE_INJECTED="$S/injected"
    set -e
    [ ! -e "$S/injected" ] && ok "hardened pin ignores DYLD_INSERT_LIBRARIES" || bad "injected"

    echo "== cost of the check (20 runs each, fresh processes, stub stops before the call)"
    times() { # times PIN ID
        for _ in $(seq 20); do
            HOME="$H" TD_PIN_SPIKE_TIMES=1 TD_PIN_SPIKE_DRY=1 "$1/Contents/MacOS/$2" 2>&1 | sed -n 's/.*check \([0-9.]*\) ms, dlopen \([0-9.]*\) ms/\1 \2/p'
        done | sort -n | awk '{c[NR]=$1; d[NR]=$2} END {printf "check p50 %.1f ms, max %.1f ms; dlopen p50 %.1f ms\n", c[int(NR/2)+1], c[NR], d[int(NR/2)+1]}'
    }
    app "$S/apps/TD.app" "$A"
    printf '  fixture, ad-hoc, dev requirement:            %s\n' "$(times "$S/pins/dev.app" dev)"
    app "$S/apps/TD.app" "$RA"
    printf '  fixture, Developer ID, team requirement:     %s\n' "$(times "$S/pins/rel.app" rel)"

    if [ "${SPIKE_APP:-}" = 1 ]; then
        echo "== today's app as a library"
        cargo build -q --release --manifest-path "$root/crates/term-pin/spike/dylib/Cargo.toml" --target-dir "$root/target/spike"
        LIB="$S/lib/today.dylib"
        cp "$root/target/spike/release/libtd_spike_dylib.dylib" "$LIB"
        install_name_tool -id @rpath/libterminal_decline.dylib "$LIB" 2>/dev/null
        cp "$LIB" "$S/lib/today-adhoc.dylib"
        codesign -f -s "$ID" -o runtime --timestamp -i "$LIBID" "$LIB" 2>/dev/null
        codesign -f -s - -i "$LIBID" "$S/lib/today-adhoc.dylib" 2>/dev/null
        printf '  library size: %s bytes\n' "$(stat -f %z "$LIB")"
        app "$S/apps/TD.app" "$S/lib/today-adhoc.dylib"
        printf '  today, ad-hoc, dev requirement:              %s\n' "$(times "$S/pins/dev.app" dev)"
        app "$S/apps/TD.app" "$LIB"
        printf '  today, Developer ID, team requirement:       %s\n' "$(times "$S/pins/rel.app" rel)"

        echo "== start to first frame (opens windows briefly), and the running pin's signature"
        now() { perl -MTime::HiRes=time -e 'printf "%.4f\n", time'; }
        # first_frame EXE: ms from exec to the first finished frame's dump
        first_frame() {
            rm -f "$S/dump.jsonl"
            local script='until [ "$(ps -o tpgid= -p $$)" -eq "$(ps -o pgid= -p $$)" ]; do sleep 0.02; done; printf "\033]7770;dump\007"; sleep 1; printf "\033]7770;quit\007"; sleep 5'
            local t0 t1
            t0=$(now)
            env HOME="$H" XDG_CONFIG_HOME="$H/.config" DECLINE_TEST_DUMP="$S/dump.jsonl" "$1" -e /bin/sh -c "$script" 2>/dev/null &
            local pid=$!
            until [ -s "$S/dump.jsonl" ]; do sleep 0.005; done
            t1=$(now)
            if [ "${2:-}" = sig ]; then
                codesign -dvvv "$pid" 2>&1 | grep -E '^(Identifier|CDHash|Signature|TeamIdentifier)=' | sed 's/^/    running pin: /' >&2
                codesign --verify "$pid" 2>&1 && echo "    running pin: codesign --verify <pid> passes (dynamically valid with the Developer ID library loaded)" >&2
                vmmap "$pid" 2>/dev/null | grep -m1 libterminal_decline | sed 's/^/    mapped: /' >&2 || true
            fi
            wait "$pid" || true
            perl -e "printf '%.0f', ($t1 - $t0) * 1000"
        }
        first_frame "$S/pins/rel.app/Contents/MacOS/rel" sig >/dev/null
        if [ -x "$root/target/release/terminal-decline" ]; then
            for _ in 1 2 3 4 5; do printf '%s ' "$(first_frame "$root/target/release/terminal-decline")"; done >"$S/static.txt"
        fi
        for _ in 1 2 3 4 5; do printf '%s ' "$(first_frame "$S/pins/rel.app/Contents/MacOS/rel")"; done >"$S/pin.txt"
        printf '  static binary, ms: %s\n' "$(cat "$S/static.txt" 2>/dev/null)"
        printf '  pin + Developer ID library, ms: %s\n' "$(cat "$S/pin.txt")"
        defaults delete red.forbes.terminal-decline.pin.rel >/dev/null 2>&1 || true
    fi
fi
echo "done: $S"
