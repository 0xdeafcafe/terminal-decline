# Pins and packaging: launcher, library, signing, pin icons (P.1–P.5)

Status: **draft, revised after an Opus (xhigh) review** (29 Oct 2026;
§0). Uses the new names (the rename, roadmap N.10, has landed;
docs/design/preferences.md): the app is `Terminal Decline.app`, bundle id
`red.forbes.terminal-decline`, command `terminal-decline` (with a
`decline` alias from the cask), config in `~/.config/terminal-decline`,
pin ids `red.forbes.terminal-decline.pin.<id>`.

The problem (DESIGN "Dock pins", known limit): a pin today is an APFS
clone of the whole binary, ad-hoc signed. An ad-hoc signature's identity
is the hash of the code, so every `pin --update` after an upgrade gives
the pin a new identity, and macOS asks again for every permission it had
(Documents, Full Disk Access, camera, notifications…). The fix is
Chrome's web-app approach: the pin holds a small **stub** that never
changes after the pin is made, and loads Terminal Decline's code from the
installed app.

Signing secrets (Developer ID, notarization) aren't in the repo yet
(roadmap N.4). Everything below builds, runs and is tested with **ad-hoc
signing first**; real signing adds a team-anchored check and
notarization without changing the layout (§4).

Modules named here:
- **term-pin** (new): the pin stub and the app's launcher, two small
  executables that find the library, check it and call into it; plus the
  frozen interface between them (`abi`).
- **term-icon** (new): generated pin icons by the Slope rule
  (`slope-pins-2`, docs/brand/README.md).
- **term-app**: the library (all of Terminal Decline), the `pin` module
  that makes, lists, edits and repairs pins, and `Paths`.
- **term-dylib** (new, a few lines): builds term-app as
  `libterminal_decline.dylib` and defines the exported entry points.
- packaging: `scripts/bundle.sh`, `scripts/dogfood.sh`,
  `packaging/entitlements*.plist`, `.github/workflows/release.yml`, the cask.

---

## 0. Revisions after the review

The reviewer tried the risky parts on this Mac (macOS 27.0, rustc 1.98.1,
SDK 27). **Measured and holding**: an ad-hoc hardened stub refuses an
ad-hoc library unless it has `disable-library-validation` ("mapping
process and mapped file (non-platform) have different Team IDs"), and
loads it with it; ad-hoc signatures carry and run the hardened runtime,
library validation off, camera, microphone, location and Apple Events
entitlements with no provisioning profile; the hardened stub ignores
`DYLD_INSERT_LIBRARIES` (a plain one honours it); signing the same input
twice gives the same bytes and code hash, and the ad-hoc identity is
`cdhash H"…"`; a program started through `forkpty` asks macOS for access
as the pin; `CFUserNotificationDisplayAlert` is not deprecated in SDK 27;
a Finder custom icon (`Icon\r` at the bundle root) leaves the code hash
unchanged and `codesign --verify --strict` passing.

| Review item | Where |
|---|---|
| The library check doesn't protect a pin's permissions from programs running as the same user (they can edit its settings or pass `-e`); pins now accept no command-choosing arguments, and the doc says what the check does protect | §1, §3.4, §3.6 |
| `pin --update` would have written the launcher into stub pins; it now leaves stub pins alone; pins from before the rename are rebuilt in one step | §5.3, §5.5 |
| The icon drawing follows `pin-template.svg` exactly (depth plane, order, light, rim, gradients per path, coordinates); palette values kept; icons kept on rename and repair | §8 |
| Rebuilding a pin swaps it atomically with `renamex_np(RENAME_SWAP)` | §6 |
| The signing spike moves first (roadmap: "spiked early") | §10 |
| Interface frozen at the first build that makes stub pins; versions never dropped; a stub tries its version down to 1 | §3.5 |
| What else the stub freezes: weak-linked LaunchServices, absolute override path, no XDG, the download URL; listed | §3.3, §3.5 |
| LaunchServices candidates in the Trash, disk images or translocation skipped; highest build wins | §3.3 |
| Signature check measured: 15–30 ms for a release library, ~4 ms ad-hoc; flags and budget updated; the library must stay a plain dylib | §3.4, §7 |
| Dogfood restarts and pins | §4.3 |
| Changing a pin's icon without a permission reset (Finder custom icon) | §5.2 |
| Quarantine removed before signing; a fresh Mach-O UUID per pin (local network privacy) | §3.1 |
| Usage strings completed; folders dropped on a pin decided before freezing | §2.3, §3.1, §11 |
| Tests made hermetic and exact; released stubs kept and run against new libraries | §9 |
| `.icns` written by ImageIO, checked by an `iconutil` round trip | §8.4 |
| Build: the stub needs its own profile; real sizes (stub ~290 KB, library 2.2 MB) | §2.5, §7 |
| Exports, install name, `[pin]` record in its own file, ABI header notes | §2.5, §3.5, §5.4 |

---

## 1. Decisions at a glance

| Question | Decision | Why |
|---|---|---|
| App layout | `Contents/MacOS/terminal-decline` is a small **launcher**; all the code is `Contents/Frameworks/libterminal_decline.dylib` (a plain dylib, never a framework); the pin stub ships in `Contents/Helpers/terminal-decline-pin` | pins and the app run the same library from the same file; checking a framework validates its whole bundle (120–180 ms measured on Ghostty's) |
| What a pin holds | the stub (copied from the app at creation), a frozen `Info.plist`, its icon; settings stay outside the bundle | nothing in the bundle changes again, so its signature and identity never change |
| Why permissions then persist, even ad-hoc | an ad-hoc pin's identity is its code hash; the stub, Info.plist and icon never change, so the hash never changes (measured: deterministic) | the fix works before the signing secrets exist |
| Pin's interface to the library | one C function per interface version, `td_pin_main_v1(const td_pin_info_v1 *)`; versions only ever added; **v1 frozen at the first build that makes stub pins** | a pin made by any public build runs every later library |
| Checking the library | before loading it, the stub checks its code signature against a requirement written into the pin (Apple anchor + Terminal Decline's team ID for release-made pins) | catches a wrong, old or damaged library, and for release pins a swapped one |
| What that does not protect | a pin's permissions from programs running as the same user: they can edit the pin's settings (its command) or its environment. Terminal.app and every terminal have the same exposure | stated, not hidden; open question 1 |
| Pin's own signing | ad-hoc, **hardened runtime**, `disable-library-validation` (so it may load the Developer ID library) plus the resource-access entitlements | measured to work ad-hoc; `DYLD_*` variables are ignored; our check replaces the system's library validation |
| Pin arguments | in the pin role the library accepts no `-e`, `--command` or `--config`; only `--restore` and `-psn_` | `open -b <pin> --args -e …` can't run something else with the pin's permissions |
| App signing | inside-out: library, Sparkle (B.4), stub, then the app with its entitlements; **no `--deep`**; hardened runtime, timestamp, notarized | Apple's rule for nested code; `--deep` signs everything with the outer options |
| Finding the app | a dev override file (absolute path), then `/Applications`, then `~/Applications`, then LaunchServices by bundle id (Trash, disk images and translocated copies skipped; highest build first) | prefers the release; a moved app is still found; never a trashed copy |
| When it isn't found or fails the check | an alert (`CFUserNotificationDisplayAlert`, no AppKit in the stub) saying what is wrong and what to do; exit 1 | an app from the Dock has no stderr |
| Bare `cargo install` | a static `terminal-decline` binary, no library; its pins keep the clone method | no app bundle to load from |
| Pin icons | generated by `slope-pins-2` in term-icon (Core Text outline of the initial, CoreGraphics drawing following `pin-template.svg`, ImageIO's `.icns` writer); what was generated, palette values included, is kept | stable across upgrades and OS font changes |
| Old pins | `pin --repair` rebuilds clone pins (from before or after the rename) as stub pins in one step; the app says so in its first window | one permission reset, chosen by the user |

---

## 2. The app bundle (P.1)

### 2.1 Layout

```
Terminal Decline.app/Contents/
  Info.plist                       CFBundleIdentifier red.forbes.terminal-decline,
                                   CFBundleExecutable terminal-decline, usage strings (§2.3),
                                   SUPublicEDKey etc. (B.4, docs/design/updater.md)
  MacOS/terminal-decline           launcher (term-pin), ~290 KB
  Frameworks/libterminal_decline.dylib   all of Terminal Decline, ~2.2 MB
  Frameworks/Sparkle.framework     B.4
  Helpers/terminal-decline-pin     the pin stub (term-pin), ~290 KB, copied into pins
  Resources/icon.icns
  Resources/dogfood                dogfood builds only (reload.rs)
  _CodeSignature/CodeResources
```

`Contents/Helpers` is where Apple's bundle layout puts helper executables;
codesign treats it as nested code.

### 2.2 The launcher

`MacOS/terminal-decline` (term-pin, `src/bin/launcher.rs`):

1. `_NSGetExecutablePath`, then `realpath` (the cask's
   `/opt/homebrew/bin/terminal-decline` and `decline` are symlinks to it;
   hardened `dlopen` refuses relative paths, measured); the library is
   `../Frameworks/libterminal_decline.dylib` from there.
2. `dlopen(RTLD_NOW | RTLD_LOCAL)`, `dlsym("td_app_main_v1")`, call it with
   `argc`/`argv`, exit with what it returns.
3. No signature check of its own: the launcher is hardened in release
   builds, so the system's library validation already requires the
   library to be signed by the same team. It ships with the app, so its
   interface to the library (`td_app_main_v1`) is not frozen.

Failure (library missing, symbol missing): message to stderr and, when
not started from a terminal (`isatty(2)` false), the alert of §3.5.

Why a launcher and a library rather than the app's executable itself:
macOS refuses to `dlopen` an executable, so the code pins load must be a
library, and the app runs the same library rather than carry the code
twice.

### 2.3 Usage strings and entitlements

Programs run in a terminal ask macOS for access as the terminal (it is
their "responsible" app), so the app must declare what any of them might
ask for. In Info.plist, each string saying "A program running in Terminal
Decline wants to …":

`NSMicrophoneUsageDescription`, `NSCameraUsageDescription`,
`NSAudioCaptureUsageDescription`, `NSAppleEventsUsageDescription`,
`NSAppleMusicUsageDescription`, `NSLocationUsageDescription`,
`NSLocationWhenInUseUsageDescription`, `NSContactsUsageDescription`,
`NSCalendarsUsageDescription`, `NSCalendarsFullAccessUsageDescription`,
`NSRemindersUsageDescription`, `NSRemindersFullAccessUsageDescription`,
`NSPhotoLibraryUsageDescription`, `NSBluetoothAlwaysUsageDescription`,
`NSLocalNetworkUsageDescription`, `NSSpeechRecognitionUsageDescription`,
`NSDesktopFolderUsageDescription`, `NSDocumentsFolderUsageDescription`,
`NSDownloadsFolderUsageDescription`, `NSRemovableVolumesUsageDescription`,
`NSNetworkVolumesUsageDescription`, `NSSystemAdministrationUsageDescription`.

`packaging/entitlements.plist` (the app, hardened): the complete list of
hardened-runtime resource entitlements:
`com.apple.security.device.audio-input`, `…device.camera`,
`…automation.apple-events`, `…personal-information.location`,
`…personal-information.addressbook`, `…personal-information.calendars`,
`…personal-information.photos-library`. Nothing that weakens the runtime
(no JIT, no unsigned memory, no DYLD variables, library validation on).

`packaging/entitlements-dev.plist` (ad-hoc builds with `HARDENED=1`): the
same plus `com.apple.security.cs.disable-library-validation`, because an
ad-hoc launcher and an ad-hoc library have no team ID to match (measured:
without it the library is refused).

### 2.4 Signing, inside-out

`scripts/bundle.sh` (rewritten in PR P-4), with `ID` = `CODESIGN_IDENTITY`
(default `-`, ad-hoc). Release adds `--options runtime --timestamp`;
ad-hoc adds `--options runtime` only with `HARDENED=1`.

```sh
codesign -f -s "$ID" $opts -i red.forbes.terminal-decline.lib  Contents/Frameworks/libterminal_decline.dylib
# B.4: Sparkle's parts, then Sparkle.framework (updater.md §5.3)
codesign -f -s "$ID" $opts -i red.forbes.terminal-decline.pin-stub Contents/Helpers/terminal-decline-pin
codesign -f -s "$ID" $opts --entitlements "$ents" "Terminal Decline.app"   # signs the launcher and seals the bundle
codesign --verify --strict --deep -vv "Terminal Decline.app"
```

- No `--deep` when signing (it would sign nested code with the outer
  identifier, entitlements and options). `--deep` when verifying checks
  every nested signature.
- The library's identifier `red.forbes.terminal-decline.lib` is part of
  what pins require (§3.4).
- Release (release.yml): notarize the zip, staple, then `spctl -a -t exec
  -vv` and `xcrun stapler validate`. Notarization needs hardened runtime
  and a timestamp on every executable, the stub included, which is why it
  is signed even though pins re-sign their copy.
- dSYMs: the release keeps `libterminal_decline.dylib.dSYM` (where the
  code is) plus the launcher's and stub's; the UUID check in release.yml
  moves to the library.

### 2.5 Crates and build outputs

| Output | Built from | Ships in |
|---|---|---|
| `libterminal_decline.dylib` | `crates/term-dylib` (cdylib; depends on term-app and **defines** the `extern "C"` entries `td_app_main_v1`, `td_pin_main_v1`; install name `@rpath/libterminal_decline.dylib`) | app |
| `terminal-decline` (launcher) | `crates/term-pin`, bin `launcher` | app |
| `terminal-decline-pin` (stub) | `crates/term-pin`, bin `pin-stub` | app, copied into pins |
| `terminal-decline` (static) | `crates/term-app`, bin, as today | `cargo install` only |

- term-pin depends on nothing but `libc` and hand-written FFI to
  CoreFoundation and Security. Cargo doesn't allow `panic` in a
  per-package profile and the workspace's release profile unwinds, so its
  two bins are built in a second cargo run with their own profile
  (`release-stub`: `panic = "abort"`, `opt-level = "z"`, fat LTO,
  stripped). Measured minimal size: ~290 KB.
- term-app's library crate name collides with the cdylib's
  (`terminal_decline`); the term-app library is renamed internally so the
  cdylib keeps the public file name. A test checks `nm -gU` on the dylib
  lists exactly the `_td_*` entry points.
- The term-app library gains `run(role, args)`, with
  `Role::{App, Pin(PinInfo), Bare}`; `main.rs` (the bare binary) calls it
  with `Bare`. `Paths` gains `app_bundle` (from the launcher's path, or
  the pin info) so the `pin` module copies the stub from
  `<app>/Contents/Helpers/` instead of cloning `exe`. P-3 lists every use
  of `NSBundle.mainBundle` and `current_exe` and decides for each whether
  it means the pin or the app (the dogfood marker, the preferences
  program's path, the version shown, crash logs).

---

## 3. The pin stub (P.2)

### 3.1 A pin bundle

```
Rush.app/Contents/
  Info.plist        frozen at creation (below)
  MacOS/rush        the stub, copied from the app's Helpers/terminal-decline-pin, then signed
  Resources/icon.icns
  _CodeSignature/CodeResources
~/.config/terminal-decline/pins/rush.toml       command, cwd, env, any setting (what --edit opens)
~/.config/terminal-decline/pins/rush.pin.toml   the [pin] record (§5.4), written by Terminal Decline
```

Info.plist keys, all written once:

| Key | Value |
|---|---|
| `CFBundleIdentifier` | `red.forbes.terminal-decline.pin.rush` |
| `CFBundleExecutable` | `rush` |
| `CFBundleName`, `CFBundleDisplayName` | `Rush` |
| `CFBundleIconFile` | `icon` |
| `CFBundlePackageType` | `APPL` |
| `CFBundleShortVersionString`, `CFBundleVersion` | the stub's version (`1`), not the app's |
| `LSMinimumSystemVersion` | `14.0` |
| `NSHighResolutionCapable`, `NSSupportsAutomaticGraphicsSwitching` | true |
| `CFBundleDocumentTypes` | folders (`public.folder`), if open question 7 says pins open dropped folders; decided before v1 is frozen |
| usage strings | as §2.3, worded "A program running in Rush (a Terminal Decline pin) wants to …" |
| `TDPinAbi` | `1` |
| `TDPinId` | `rush` |
| `TDAppBundleId` | `red.forbes.terminal-decline` |
| `TDLibraryRequirement` | the code requirement the library must meet (§3.4) |

P-1 goes through the app's Info.plist key by key and records, for each,
whether pins carry it; anything added to pins later means a permission
reset for every pin, so the list is decided once.

Making the bundle, in order:
1. Copy the stub (`cp -c`), Info.plist and icon into a temporary
   directory named without `.app` (so LaunchServices never registers a
   half-made pin).
2. Remove `com.apple.quarantine` (measured: `cp -c` copies it from a
   cask-installed app, which would leave the pin blocked as
   unnotarized).
3. Write a fresh `LC_UUID` into the stub. Every pin from one build would
   otherwise share the stub's Mach-O UUID (measured), and macOS tracks
   local network permission by the main executable's UUID (TN3179).
4. Sign once:
   `codesign -f -s - -o runtime --entitlements <pin entitlements> -i red.forbes.terminal-decline.pin.rush`,
   with the pin entitlements: the resource-access entitlements of §2.3
   plus `com.apple.security.cs.disable-library-validation`.
5. Move into place (§6), register with LaunchServices.

Ad-hoc signing is deterministic, and nothing is signed again, so the
pin's code hash — its identity to macOS's privacy settings — never
changes. (codesign versions write different signature formats, 20500 on
macOS 27 against 20400 on older copies, which matters only if something
re-signs a pin.)

### 3.2 What the stub does

1. Read its own Info.plist (`CFBundleGetMainBundle`,
   `CFBundleGetValueForInfoDictionaryKey`): `TDPinAbi`, `TDPinId`,
   `TDAppBundleId`, `TDLibraryRequirement`. Missing or wrong type: the
   alert "this pin is damaged; run `terminal-decline pin --repair`".
2. Find the app (§3.3), checking each candidate (§3.4) until one passes.
3. `dlopen` the library, look up `td_pin_main_v<N>` from its own version
   down to 1 (§3.5), call it, exit with what it returns.

### 3.3 Finding Terminal Decline.app

In order; the first candidate that passes §3.4 wins:

1. **Dev override**: the file `~/.config/terminal-decline/pin-app` (one
   line, an absolute app path, `realpath`ed; `HOME` from the environment,
   else `getpwuid`; `XDG_CONFIG_HOME` is ignored, since a Dock launch
   doesn't have it). Written by `scripts/dogfood.sh`. It only changes
   where to look, not what passes.
2. `/Applications/Terminal Decline.app`
3. `~/Applications/Terminal Decline.app`
4. LaunchServices: `LSCopyApplicationURLsForBundleIdentifier(TDAppBundleId)`,
   skipping copies in a Trash, under `/Volumes` (disk images), under
   App Translocation and in `~/Downloads`; among the rest, the highest
   `CFBundleVersion` first. This finds an app that was moved or renamed.
   The function is marked "to be deprecated" in SDK 27, so the stub looks
   it up with `dlsym` (only CoreFoundation is linked strongly): if a
   future macOS removes it, this step is skipped instead of the stub
   failing to start.

A candidate must have `Contents/Info.plist` with `CFBundleIdentifier` =
`TDAppBundleId` and `Contents/Frameworks/libterminal_decline.dylib`. The
reason each one failed is kept for the alert. `pin --list` shows which app
each pin would use.

### 3.4 Checking the library's signature

`SecStaticCodeCreateWithPath(dylib)`, `SecRequirementCreateWithString(TDLibraryRequirement)`,
`SecStaticCodeCheckValidityWithErrors(code, kSecCSStrictValidate |
kSecCSNoNetworkAccess, requirement)`. The release is arm64 only, so all
architectures aren't checked.

The requirement is written into the pin when it is made, from how the
app making it is signed (the library reads its own signing information
with `SecCodeCopySigningInformation`):

- **Release-made pin** (the library has a team ID):
  `anchor apple generic and certificate leaf[subject.OU] = "<TEAMID>" and identifier "red.forbes.terminal-decline.lib"`.
  This also accepts Apple Development certificates of the same team; if
  dogfood builds won't use one (open question 2), the Developer ID
  certificate's marker is added to the requirement.
- **Dev pin** (the library is ad-hoc): `identifier "red.forbes.terminal-decline.lib"`.
  Any ad-hoc library with that identifier passes, so it guards against
  mistakes, not attacks. `pin --list` marks these pins "dev".

**What the check protects, and what it doesn't.** It makes sure a pin
runs Terminal Decline's library, of a version that understands the pin,
undamaged, and (for release pins) signed by our team, so dropping another
library into an unprotected copy of the app doesn't run inside the pin.
It does **not** stop a program running as the same user from borrowing a
pin's permissions: such a program can change the pin's settings (its
command and environment) or its config, and the pin then runs that, with
its grants. That is the same exposure every terminal has (whatever runs
in Terminal.app has Terminal.app's Full Disk Access). Pins remove the
easiest way in — in the pin role, the library accepts no `-e`,
`--command` or `--config` arguments — but the settings file remains.
Open question 1.

Check then load is not atomic (macOS has no `dlopen` by descriptor; P-0
tries `dlopen("/dev/fd/N")` of the checked file). Writing into a notarized
app bundle in `/Applications` is guarded by App Management (§6), but
dogfood and ad-hoc builds aren't protected, and many users allow their
terminal App Management for `brew`; given the paragraph above, this is
accepted.

**Cost, measured** as the stub would pay it, first call in a fresh
process:

| Library | Requirement | Time |
|---|---|---|
| Developer ID, 3.9 MB universal | team | 15–30 ms (one cold run 128 ms) |
| same, with `kSecCSNoNetworkAccess` | team | 15–16 ms |
| ad-hoc, 2.2 MB | identifier | ~4 ms |

### 3.5 The frozen interface

`crates/term-pin/abi/terminal_decline_pin.h`, mirrored by a `#[repr(C)]`
struct in `term_pin::abi`:

```c
/* Frozen from the first build that makes stub pins. New versions add a
   new struct and a new function; these are never changed or removed.
   All pointers stay valid for the life of the process; paths are bytes
   (not necessarily UTF-8); the function is called on the main thread. */
#define TD_PIN_ABI_V1 1

struct td_pin_info_v1 {
    uint32_t size;               /* sizeof(struct td_pin_info_v1): a check, not a version */
    uint32_t abi;                /* 1 */
    const char *pin_id;          /* "rush" */
    const char *pin_bundle;      /* path of the pin's .app */
    const char *app_bundle;      /* path of the Terminal Decline.app the library came from */
    const char *stub_version;    /* the stub's CFBundleShortVersionString */
    int argc;
    const char *const *argv;
};

/* Returns the process's exit status (it may also exit itself). */
int td_pin_main_v1(const struct td_pin_info_v1 *info);
```

- **One versioning rule**: each version is its own function and struct.
  A stub of version N tries `td_pin_main_vN`, then N − 1, … down to 1, so
  a newer pin still runs on an older app where it can. The library
  exports every version that has ever been in a public build and never
  drops one. `size` only guards against a mismatched header.
- **Frozen from the first public build that makes stub pins** (the 0.1.0
  beta, if P lands before it), not from 1.0: beta users' pins must not
  lose their permissions to an interface change.
- Everything else the stub freezes is listed next to the header and
  never changed: the Info.plist keys of §3.1, the search order and paths
  of §3.3, the override file's place, the requirement forms of §3.4, the
  strongly linked frameworks (CoreFoundation, Security; the rest through
  `dlsym`), the alert texts and the download URL (open question 8).
- Apple events (open a document or URL, reopen) reach the pin's
  `NSApplication`, which the library creates, so they need nothing in the
  interface. The entry catches panics (`catch_unwind`) so none crosses
  into C.
- A pin needing a version the library doesn't have: the alert says "this
  pin needs a newer Terminal Decline; update it from the Terminal
  Decline menu, or `brew upgrade --greedy terminal-decline`".

Alerts: `CFUserNotificationDisplayAlert` (CoreFoundation, so the stub
needs no AppKit), title "Rush can't start", the reason (app not found;
library fails its check, naming the path; interface missing), and two
buttons: "Get Terminal Decline" (opens the download page) and "Quit". The
same text goes to stderr.

### 3.6 Inside the library, as a pin

`td_pin_main_v1` checks `size`/`abi`, builds `Role::Pin(PinInfo)` and runs
the app as a pin does today (the pin's settings layered on the user's
config; DESIGN "Dock pins"), with only `--restore` and `-psn_` accepted as
arguments. `NSBundle.mainBundle` is the pin's bundle, so the Dock icon,
Cmd+Tab entry, notification permission and Secure Keyboard Entry setting
stay the pin's own. A pin never loads Sparkle (B.4). A running pin that
starts another program from the app (Cmd+, runs the preferences program,
`pin` commands) runs the installed app's current binary, which may be
newer than the library the pin loaded; those programs' arguments stay
compatible.

---

## 4. Ad-hoc first, real signing later

### 4.1 What works with ad-hoc signing only (measured in §0)

- The whole layout, the launcher, the stub and its interface.
- Pin identity stays the same across app upgrades: the pin is ad-hoc
  anyway, and its bundle never changes. This is P.2's "done when", and an
  automated test checks it (§9).
- The library check in its dev form.
- Hardened pins (ad-hoc signatures carry the runtime and these
  entitlements), so `DYLD_INSERT_LIBRARIES` is ignored.

### 4.2 What needs a certificate

- The app's Developer ID signature, notarization, stapling (N.4's
  secrets).
- Release-made pins with the team requirement. A free Apple Development
  certificate (any Apple ID has one) matches `anchor apple generic` plus
  a team, so the team requirement and the permission behaviour can be
  spiked in P-0 before N.4; only notarization waits.
- The P.1 check "a program asking for the mic gets a prompt" in a
  notarized build.

### 4.3 Dogfooding

Alex's daily app is `~/Applications/Terminal Decline.app` from
`scripts/dogfood.sh`, which also writes `~/.config/terminal-decline/pin-app`
so his pins load the dogfood build. Pins he makes from an ad-hoc dogfood
build are dev pins.

Ad-hoc identity is the code hash, so today every dogfood install is a new
app to macOS, and the app's own permissions reset each time. Signing
dogfood builds with a certificate (Apple Development or Developer ID, when
`security find-identity -v -p codesigning` finds one) would stop that and
make his pins release-grade (open question 2).

Restarting into a new dogfood build (reload.rs): `own_bundle()` uses
`current_exe`, which in a pin is the pin's bundle (no dogfood marker), so
running pins keep the old, deleted library until they are restarted by
hand. Making pins restart too needs one restore file per bundle id (today
all would share `~/Library/Caches/terminal-decline/restore.json`) and a
relaunch with `open -n <pin>`. Open question 6.

### 4.4 Moving to real signing

Nothing in the layout changes. bundle.sh gets the identity from the
release workflow (it already imports the certificate when the secrets
exist); pins made from then on carry the team requirement. Dev pins keep
working with dev builds; `pin --repair --release` turns a dev pin into a
release-made one (its Info.plist changes, so permissions are asked once
more). The team ID is bound into release pins for good (open question 3).

---

## 5. Pin commands (P.4) and migration

### 5.1 `pin --list`

```
using /Applications/Terminal Decline.app (0.4.0, build 1234)
NAME     STATUS                 COMMAND              PIN
Rush     ok                     /Users/me/rush       ~/Applications/Rush.app
htop     ok (dev pin)           htop                 ~/Applications/htop.app
Logs     old: run --repair      tail -f …            ~/Applications/Logs.app
Claude   pin moved: --repair    claude               (was ~/Applications/Claude.app)
```

Statuses, worked out without starting the pin:
- `ok` / `ok (dev pin)`;
- `old: run --repair`: a clone pin (`TDPinAbi` absent), from before or
  after the rename;
- `clone`: a pin made by a bare `cargo install` binary (§5.6);
- `needs a newer app`: no interface version the pin can use;
- `damaged`: `SecStaticCodeCheckValidity` on the pin fails, or its
  Info.plist is unreadable;
- `pin moved: --repair`: `pin_app` in its record doesn't exist, but
  LaunchServices knows the pin's bundle id elsewhere (shown);
- `missing`: neither.

`--list --json` gives the same for scripts and the preferences tab's Pins
section (E.18). The app line at the top is found the way the stub finds
it, so a moved app is visible.

### 5.2 `pin --edit NAME`, and changing a pin

`pin --edit NAME` opens `pins/<id>.toml` in `$VISUAL`, else `$EDITOR`,
when run in a terminal, else `open -t`. Afterwards the file is checked
with the config loader and errors are printed. Only the settings file is
touched, so permissions are kept; the running pin reloads it (live reload
already watches pin settings).

Running `pin Rush -- ./other` on an existing pin rewrites only its
settings, not its bundle.

**A new icon** (`pin Rush --icon FILE` or `--regenerate-icon` on an
existing pin) is set as a Finder custom icon (`NSWorkspace setIcon:forFile:`,
which writes `Icon\r` at the bundle root). Measured: the code hash stays
the same and `codesign --verify --strict` still passes, so permissions are
kept. P-6 checks how the Dock, Finder and Cmd+Tab show it (open question
4). The icon inside the bundle stays as created.

**A rename** rebuilds the bundle (a new name is a new Info.plist), keeping
its icon and `[pin.icon]` record, and changes its identity, so it asks
first: "macOS will ask again for any permissions Rush has (camera,
folders, …). Continue? [y/N]" (`--yes` to skip).

### 5.3 `pin --repair [NAME]` and `pin --update`

`--repair`, for each pin (or the one named):
- registers it with LaunchServices again (`LSRegisterURL`, which also
  makes it show in Spotlight);
- `pin moved`: updates `pin_app` in its record to where LaunchServices
  finds it;
- `old` clone pins, whether `red.forbes.decline.pin.*` from before the
  rename or `red.forbes.terminal-decline.pin.*` after it: rebuilds the
  bundle as a stub pin in place, with the current id prefix, keeping its
  name, icon file and settings, in **one** rebuild, after the confirmation
  of §5.2;
- `damaged`: rebuilds, with the confirmation.

"Repair if the app moved" needs nothing in the pin itself: the stub finds
a moved app through LaunchServices (§3.3). `--repair` is for pins that
moved, and for old pins.

`--update` (kept, since dogfood.sh, the cask caveat and users' habits call
it) re-clones **clone pins only**, as today; for stub pins it prints
"Rush is up to date (it loads the installed Terminal Decline)" and
changes nothing. A test checks `--update` never writes into a stub pin.
dogfood.sh and the cask caveat change to `--repair` in P-4.

### 5.4 Migration from clone pins

- The first window after an upgrade to the stub layout shows a notice (as
  config notices are shown): "3 pins still run their own copy of an older
  version. Run `terminal-decline pin --repair` to update them; macOS will
  ask once more for their permissions." Shown once per launch while any
  remain. Nothing is rebuilt without the user asking, because rebuilding
  resets permissions and may raise App Management prompts (§6).
- Clone pins keep working until repaired: they contain a whole older
  binary.

The pin's record lives in its own file, `pins/<id>.pin.toml`, written only
by Terminal Decline (so `--edit` doesn't show it, and an older library
doesn't report its keys as unknown settings):

```toml
stub = 1                      # interface version of the stub inside
method = "stub"               # or "clone" (bare binary pins)
made_by = "0.4.0 (1234)"      # app version that made it
requirement = "release"       # or "dev"
pin_app = "/Users/me/Applications/Rush.app"
[icon]                        # §8; absent with --icon
generator = "slope-pins-2"
canonical = "rush"
hash = 20943315
index = 5
top = "#205B51"
bottom = "#102D28"
initial = "R"
outline = "M…Z"               # the outlined initial, 1024 grid, top-down
```

### 5.5 dogfood.sh after stubs

dogfood.sh calls `pin --update` today (for the rename). From P-4 it calls
`pin --repair` only when `--list --json` reports old pins, and writes the
`pin-app` file.

### 5.6 Pins from a bare binary

`cargo install` builds a static `terminal-decline` with no app bundle
around it, so there is no library to load. Its pins keep today's method
(clone the binary, `method = "clone"`) and `pin --update` re-clones them,
asking again for permissions as today. `pin --list` shows them as `clone`.

### 5.7 From Finder, Spotlight, Login Items

A pin is a normal app: registered with LaunchServices at creation, it can
be opened from Finder, Spotlight, `open -a Rush`, `open -b
red.forbes.terminal-decline.pin.rush`, and added to Login Items in System
Settings by hand. Each way starts it with no terminal environment (cwd
`/`, a launchd `PATH`), which the pin already handles (login shell, cwd
default; DESIGN "Dock pins"). These are smoke-tested (§9). Making a pin
from an app that runs translocated or from a disk image prints a warning,
since the stub won't find that copy later.

---

## 6. Rebuilding atomically, and App Management prompts (P.5)

A rebuild writes the new bundle under a temporary name without `.app`
(§3.1), then swaps it with the old one in one call
(`renamex_np(new, old, RENAME_SWAP)`, same volume; plain `rename` can't
replace a non-empty directory), then deletes the old copy. A refusal at
any point leaves the old pin working. A running pin is asked to quit
first (or the user is told to restart it afterwards).

macOS 13 and later protect app bundles from being changed by other apps
("App Management" in Privacy & Security). What triggers it for us is to be
measured, not assumed. Known so far:

- The `pin` command runs inside a terminal, so macOS attributes its file
  changes to that terminal (Terminal Decline, Terminal.app, iTerm2…);
  any prompt names that app.
- Creating a new bundle in `~/Applications` is creating files, not
  changing an app; we expect no prompt.
- Changing or deleting an existing pin (rebuild, `--remove`) changes an
  app bundle; App Management is thought to cover notarized, team-signed
  apps, so ad-hoc, never-quarantined pins may be outside it — the main
  thing to find out.
- Sparkle replacing Terminal Decline.app is the same team changing its
  own app, which macOS allows (B.4).

PR P-6 runs this matrix on macOS 14, 15, 26 and 27 (in VMs, plus Alex's
Mac; GitHub runners can't answer prompts) and records the result in
DESIGN.md: {create, rebuild, set icon, remove} × {`~/Applications`,
`/Applications`} × {run in Terminal Decline (ad-hoc, then signed), in
Terminal.app, from the preferences tab, from inside a pin}. Where a prompt
appears, the command catches `EPERM`, says what macOS wants ("allow
Terminal Decline under System Settings › Privacy & Security › App
Management, or remove Rush.app in Finder"), and changes nothing.

---

## 7. Costs, and how to measure them

| What | Expected | How measured |
|---|---|---|
| Pin launch, start to first frame | today + ≤ 35 ms for a release pin (check 15–30 ms, `dlopen` 2–5 ms), + ≤ 10 ms for a dev pin | `scripts/pin-launch.py` (new): `open -n` a pin, 20 runs, each a fresh process, cold (after `purge`) and warm, offline and online, until the smoke `dump` control answers; p50/p95 before and after (in P-6, once P-3–P-5 exist) |
| Signature check alone | 15–30 ms release, ~4 ms ad-hoc (measured, §3.4) | a term-pin test running the stub's check in separate processes |
| App launch | unchanged ± 5 ms (one `dlopen`, no check) | same script on the app |
| Footprint of a pin, idle | unchanged ± 0.5 MB; what to watch is the library's dirty data and the stub's own Rust runtime (clean library code is shared and isn't counted) | `/usr/bin/footprint -p <pid>` on a pin before/after, standard window size |
| Disk per pin | ~300 KB stub (signing rewrites it, so the clone shares nothing) + icon 0.2–0.6 MB | `du -sk` of a new pin |
| App bundle | + ~600 KB (launcher and stub) | `du -sk`, zip size in the release job summary |
| Library | ≈ today's 2.2 MB | `ls -l` |
| Icon generation | ≤ 150 ms per pin | timed in `pin` create |
| Idle | 0% CPU, 0 wakeups | `top -l 2 -s 5 -stats cpu,idlew` on a pin |

---

## 8. Pin icons (P.3)

`docs/brand/README.md` ("Generation contract: `slope-pins-2`") and
`docs/brand/pin-template.svg` are the specification; this is how
term-icon implements them.

### 8.1 Interface

```rust
pub struct PinIcon {
    pub generator: &'static str,  // "slope-pins-2"
    pub canonical: String,        // step 1
    pub hash: u32,                // step 2, FNV-1a 32
    pub index: u8,                // hash % 10
    pub top: [u8; 3], pub bottom: [u8; 3], // the palette entry's values, kept (step 5)
    pub initial: String,          // step 3, uppercased cluster or "#"
    pub outline: String,          // the initial's outline, 1024 grid, top-down, SVG path data (numbers only)
}
pub fn plan(display_name: &str) -> PinIcon;               // steps 1–3 except the outline
pub fn outline(initial: &str) -> Result<String, IconError>; // Core Text
pub fn render_icns(icon: &PinIcon) -> Vec<u8>;            // CoreGraphics + ImageIO
```

### 8.2 Steps 1–3

- **Canonical name**: NFC (`unicode-normalization`), trim exactly the
  White_Space code points the brand doc lists (hard-coded, with a test per
  code point), fold ASCII A–Z only.
- **Hash**: FNV-1a 32 over the UTF-8, `index = h % 10`, the palette table
  copied from the brand doc in its order (a test checks the order and the
  hex values against the doc's table).
- **Initial**: from the NFC, trimmed original: the first extended grapheme
  cluster (`unicode-segmentation`) containing a Letter or Number
  (`unicode-properties` general category), uppercased with Rust's
  `to_uppercase` (full result kept: "ß" → "SS"); none: `#`.
- **Unicode version**: the contract names Unicode 15.1, which can't be
  met exactly: `to_uppercase` follows the Rust toolchain's tables and
  term-core already uses 18.0. Everything generated is kept (§5.4) and
  never worked out again, so a table difference can only affect new pins
  whose names use characters added after 15.1. Open question 5.

### 8.3 The initial's outline

Core Text: a `CTLine` of the initial in the bold system font
(`CTFontCreateUIFontForLanguage(kCTFontUIFontEmphasizedSystem, 100,
"en")`, which is `.SFNS-Bold`; a fixed language, so the fallback for CJK
initials doesn't depend on the user's locale; at 100 pt, since below
28 pt the fitted outline drifts from the template by ~0.6 px, measured),
system fallback for other scripts; each run's glyphs through
`CTFontCreatePathForGlyph` at their positions; the union's ink bounds fitted
proportionally into 400 × 360 centred at (512, 445), in the brand doc's
top-down coordinates (CoreGraphics is bottom-up; the outline is stored
top-down so it can be compared with the template). A glyph with no
outline (a colour emoji, e.g. keycap "1️⃣" whose cluster contains a
Number) falls back to the cluster's first Letter or Number scalar alone,
then to `#`. User text never goes into any SVG or markup.

### 8.4 Drawing, following pin-template.svg

From vectors at every size (no downscaling), sRGB, RGBA premultiplied,
transparent outside the body. In the 1024 grid, top-down, in this order:

1. **Body**: rounded square 824 × 824 at (100, 100), radius 185; linear
   gradient from the palette entry's top to bottom, from (0, 0) to
   (.7, 1) of the body's box.
2. **Depth plane**: a copy of the foreground (initial and Slope) moved
   8 px down, black at 22%, between the body and the foreground.
3. **Foreground**: the initial and the fixed Slope mark (`M326 702 L452
   814 Q460 822 474 822 H704`, stroke 64, round caps and joins), each with
   the gradient `#FFFFFF` → `#B9E5CC` from (0, 0) to (.3, 1) of **its own
   box** (the initial's ink box; for the Slope its geometry box
   326–704 × 702–822), drawn through that box's transform so it matches
   SVG's object-bounding-box gradients, skew included.
4. **Light**, over the foreground: a radial gradient centred at (.25, 0)
   with radius 1 of the 818 × 818 rect at (103, 103), radius 182, white
   at 6% fading to 0.
5. **Rim**, over everything: a 6 px stroke of that same rect, a linear
   gradient (0, 0) → (.3, 1), white at 55% / 6% / 22% at stops 0 / .5 / 1.

The `.icns` is written with ImageIO's own writer (`CGImageDestination`
of type `com.apple.icns`) from the rendered sizes 16, 32, 64, 128, 256,
512 and 1024 px, which produces the entries macOS's `iconutil` writes
(`ic04`, `ic05` as ARGB; `ic07`–`ic14` as PNG; measured on macOS 27). A
test round-trips the file through `iconutil -c iconset` and checks every
size; whether ImageIO's output is byte-for-byte the same for the same
input is checked in P-2 (if not, the test compares pixels).

### 8.5 Kept, and collisions

What `plan` and `outline` produced goes into `[icon]` of the pin's record
(§5.4), palette values included, and is kept on rename and repair; it is
worked out again only with `--regenerate-icon`, as step 5 of the contract
asks. Two pins can share an initial and a hue; `pin NAME --glyph X` (a
custom initial, kept the same way) or `--icon FILE` gives one of them its
own.

### 8.6 macOS 26/27

macOS 26 introduced layered "Liquid Glass" icons and Dark, Clear and
Tinted icon modes; flat `.icns` icons are shown in a way that is still to
be checked (they may be put on a plate). Pins can't use Icon Composer's
compiled format (it needs Xcode at pin creation). P-6 checks how a
generated pin looks in the Dock, Finder and Cmd+Tab on macOS 26 and 27,
in each icon mode, light and dark, with screenshots; open question 4.

---

## 9. Tests

**term-pin** (macOS CI; hermetic through `HOME` and a test-only app
bundle id):
- A fixture library: a 10-line C file compiled by the test with `xcrun
  clang -dynamiclib`, exporting `td_pin_main_v1` that writes every field
  of the info struct to a file named in `argv`, ad-hoc signed with the
  library identifier.
- Test pins carry their own `TDAppBundleId` (`red.forbes.terminal-decline.test.<random>`),
  so LaunchServices, which is system-wide, can never find a real
  installed app (on a developer's Mac it would find the dogfood build).
  A fake app in `$HOME/Applications/Terminal Decline.app` and a pin made
  by a test helper; run the stub; check the file. Then:
  - the override file wins over `~/Applications`; a relative override is
    refused;
  - wrong `CFBundleIdentifier` in the candidate: skipped;
  - library with another identifier, or one byte changed after signing:
    refused, exit 1, stderr names the reason and the path;
  - a fixture exporting only `td_pin_main_v2` with a v1 stub: the "needs
    a newer Terminal Decline" message; a v2 stub against a v1-only
    fixture: runs v1;
  - no app anywhere: the "isn't installed" message;
  - `DYLD_INSERT_LIBRARIES` set: ignored;
  - the struct layout matches the C header (a compile-time check plus the
    fixture reading it).
- The launcher: finds the library through a symlink to itself; missing
  library: message.
- **Identity across upgrades** (automated, ad-hoc): make a pin from app
  build A, record its code hash (`codesign -dv`), install build B (a
  different library), start the pin: same code hash, and the fixture file
  shows B's library ran.
- **Released stubs**: every stub binary that has been in a public build is
  kept (release assets) and a CI job runs each against the current
  library. This is the real test of "frozen".
- LaunchServices fallback with a real move: manual (§9 manual list).

**term-icon** (macOS CI):
- The brand doc's check values (Rush 20943315 → 5, htop 3212443446 → 6,
  Claude 3791422743 → 3, Logs 3473062022 → 2) for canonical text, hash
  and index.
- Roadmap P.3's eight names give distinct (initial, hue): Rush (R, 5),
  htop (H, 6), Claude (C, 3), Logs (L, 2), Vim (V, 9), Kube (K, 0), Psql
  (P, 7), Docker (D, 1).
- Canonicalisation: every listed White_Space code point trimmed, internal
  spaces kept, NFC (`e` + U+0301 → `é`), only ASCII folded (`ÉCLAIR` keeps
  `É`).
- Initial: `"  42 things"` → `4`, `"🚀 Rocket"` → `R`, `"ßeta"` → `SS`,
  `"!!!"` and `""` → `#`, keycap emoji → its digit.
- Outline: `outline("R")` matches the template's R (ink x 367.925–656.075,
  y 265–625) within 0.01.
- Drawing: colours at interior points (inside the body away from the rim,
  inside the initial) equal what the gradient formulas give there (± 2 per
  channel at 1024); corners transparent; the 128, 32 and 16 px renders of
  Rush, htop, Claude and Logs compared with crops of `docs/brand/pins.png`
  (a small per-pixel tolerance); the `.icns` round trip of §8.4.

**term-app `pin`** (existing tests carried over, plus): stub pins created
with every Info.plist key of §3.1; quarantine removed and `LC_UUID`
changed before signing; the requirement chosen by the library's own
signing information (faked through the runner); codesign called with `-o
runtime`, entitlements and the pin identifier and never `--deep`; the
rebuild swap leaves the old pin on failure; `--list` statuses from fake
bundles of each kind; `--repair` updates a moved pin and rebuilds a
pre-rename clone pin in one step; `--update` never writes into a stub
pin; `--edit` runs the editor through the runner and reports config
errors; a pin given `-e` refuses it; removal still refuses anything that
isn't a pin.

**Packaging** (macOS CI, ad-hoc): `scripts/bundle.sh` builds the app;
`codesign --verify --strict --deep` passes; each nested item has its own
identifier (`codesign -dv`); with `HARDENED=1` the entitlements match
`packaging/entitlements-dev.plist`; `plutil` finds every usage string;
`nm -gU` on the library lists exactly the `_td_*` entries. The smoke tests
gain a run **through the bundled app** (launcher plus library), not only
the bare binary, and, after P-5, a pin made from that app opening and
running a command.

**Release** (once the secrets exist): notarization succeeds, `spctl` and
`stapler validate` pass in release.yml.

**Manual, recorded in DESIGN.md**:
- P-0's spike (§10).
- P.1: in the notarized app, a program asking for the microphone (a
  three-line Swift script using `AVCaptureDevice.requestAccess`) gets
  macOS's prompt naming Terminal Decline.
- App moved to another folder: pins still start (LaunchServices).
- Pins from Finder, Spotlight, `open -b`, and as a login item after a
  restart.
- §6's App Management matrix; §8.6's icon look; §5.2's Finder custom
  icon in the Dock and Cmd+Tab.

---

## 10. PR plan

New crates make most of this parallel. No two parallel PRs touch one
crate.

| # | Crate | PR | Size | After |
|---|---|---|---|---|
| P-0 | pin (new `term-pin`) | **spike first** (roadmap: "spiked early with real signing"): the ABI header and `term_pin::abi`; a minimal stub plus today's code built as a dylib; with a free Apple Development certificate: a Documents or Full Disk Access grant survives swapping the library, a child asking for the mic gets a prompt naming the pin and the answer is kept, the team requirement passes; `dlopen("/dev/fd/N")` tried; results in DESIGN.md before building on them | S | — (needs Alex's Apple ID for the certificate; the ad-hoc parts are already measured) |
| P-1 | pin | stub and launcher in full, the search order, the signature check, alerts, the `release-stub` profile; tests with the C fixture, the identity test | M | P-0 |
| P-2 | icon (new `term-icon`) | `slope-pins-2`: plan, outline, drawing, ImageIO `.icns`; tests of §9 | M | — |
| P-3 | app (+ new `term-dylib`) | `run(role, args)`, `Role`, the entry points in term-dylib with panic catching, the pin role's argument rule, `Paths::app_bundle`, the `mainBundle`/`current_exe` audit, crash logs and dSYM naming, the library crate rename | M | P-0 (header) |
| P-4 | packaging + ci | bundle.sh rewrite (layout, inside-out signing, entitlements files, usage strings, `HARDENED=1`), the stub build, dogfood.sh (`pin-app` file, `--repair` instead of `--update`, certificate signing if present), release.yml (three outputs, dSYMs, notarize all, released stubs kept), cask caveat; the packaging CI checks and smoke tests through the bundled app | M | P-1, P-3 |
| P-5 | app | the `pin` module on stubs: create (quarantine, `LC_UUID`, frozen Info.plist, entitlements, swap), requirement from own signature, `--list` (+ `--json`), `--edit`, `--repair`, `--update` for clone pins only, rename, icons through term-icon and the Finder custom icon, the `.pin.toml` record, the migration notice, `LSRegisterURL`; align preferences.md's Pins section (icons keep permissions) | M | P-1, P-2, P-3 (parallel with P-4) |
| P-6 | docs + checks | App Management matrix (P.5), icon look on macOS 26/27, launch-time numbers, the pin-in-bundle smoke test, the real-signing checks once N.4 lands; DESIGN.md "Dock pins" (with §3.4's statement of what is and isn't protected) and "Modules" rewritten | S | P-4, P-5 |

P-0 first; then P-1, P-2 and P-3 at once; then P-4 and P-5 at once; then
P-6. P-2 can start any time.

---

## 11. Open questions (need Alex)

1. **Borrowed permissions.** A program running as you can borrow a pin's
   permissions (Full Disk Access, say) by editing the pin's settings file,
   as anything run in Terminal.app can use Terminal.app's. Acceptable? If
   not, a pin's command must live in its frozen bundle, and changing the
   command resets its permissions.
2. **Dogfood signing.** Sign dogfood builds with a certificate (which also
   stops the app's own permissions resetting on every dogfood install, and
   makes your pins release-grade)? Developer ID only, or Apple Development
   too? This decides the requirement string pins carry.
3. **Team ID.** Is the Apple team final (personal or a company account)?
   Release pins are bound to it for good.
4. **Changing a pin's icon.** The design uses a Finder custom icon
   (permissions kept, measured); if the Dock or Cmd+Tab don't show it
   well, the fallbacks are a rebuild (permissions reset) or an icon shown
   only while the pin runs. And if flat `.icns` pins sit on a plate on
   macOS 26/27, accept it or change the pin artwork?
5. **Unicode version** for pin names: the contract's 15.1 can't be met
   exactly (uppercasing follows Rust's tables); accept current tables,
   since results are kept per pin, and change the brand doc's wording?
6. **Pins and dogfood restarts.** Should running pins restart into a new
   dogfood build, as the app does?
7. **Folders dropped on a pin's Dock icon** (E.10): should a pin open a
   window there? It must be decided before the pin's Info.plist is frozen.
8. **The download URL** in the stub's alert must still work in ten years.
   The GitHub repository page, or an address of your own?
9. **Freezing the interface at the first public beta** (0.1.0) rather than
   at 1.0, so beta users' pins keep their permissions?
10. **Hardened pins** (a frozen entitlement list; injection through
    `DYLD_*` blocked) or plain ones (nothing about entitlements frozen;
    little lost while question 1 stands)? The design picks hardened.
