# Updater: Sparkle 2, silent, in the app only (B.4)

Status: **draft, revised after an Opus (xhigh) review** (29 Oct 2026;
§0). Names as renamed (N.10, landed): the app is `Terminal Decline.app`,
bundle id `red.forbes.terminal-decline`. Builds on the bundle layout of
docs/design/pins.md (launcher + `libterminal_decline.dylib`, inside-out
signing, `Role::{App, Pin, Bare}`).

What the roadmap asks (B.4): Sparkle 2 for everyone (the cask declares
`auto_updates true`, so `brew upgrade` leaves the app to update itself);
only in Terminal Decline.app's own process, never in pins; a silent
driver (no windows, no prompts); no system profiling; a daily check, with
DESIGN.md's "no timers" goal updated to say so; the EdDSA key backed up.
The release workflow publishes the appcast; the private key is the
repository secret `SPARKLE_ED_PRIVATE_KEY`.

Modules named here:
- **term-app**: the `update` module (deciding whether to run, loading
  Sparkle, the silent driver and delegate, the `update` setting, the
  Check for Updates menu item).
- packaging: `scripts/sparkle.sh` (new: fetches a pinned Sparkle release),
  `scripts/bundle.sh` (embeds and signs Sparkle), `scripts/appcast.py`
  (new: writes the appcast), `.github/workflows/release.yml`, the cask.

---

## 0. Revisions after the review

The reviewer checked every claim against Sparkle's source (2.x branch,
release 2.10.1: the update driver, updater, settings, validator,
installer launcher, plain installer, progress app, timer and
`sign_update`).

| Review item | Where |
|---|---|
| A background "update found" call comes only when Sparkle wants attention (critical, information-only, major upgrade, admin password needed, an update waiting a week), often at the "installing" stage, where **Install means quit and relaunch now** — which would kill every shell. Background calls now always dismiss; replies follow Sparkle's own state | §3.2 |
| Sparkle accepts an update whose EdDSA signature is right **whatever its code signature** (ad-hoc included). Updates are published only for notarized Developer ID builds, checked by the release job; the updater is off in ad-hoc builds | §3.5, §6.1, §2 |
| `sign_update`'s verify mode can't catch a key mismatch; the job compares public keys itself | §6.1 |
| The appcast wasn't in the uploaded artifact; a tag build without the key must fail once updates have started | §6.1 |
| A second app process (`terminal-decline -e …` from a shell) would run a second updater; one process holds a lock | §2 |
| `Updater.app` is required; removing XPC services means removing the symlink too; `Autoupdate`'s team must match for an atomic swap | §5.3 |
| Sparkle's own scheduler is a one-shot wall-clock timer with no wakeups between checks; we use it instead of our own | §4 |
| Permission reply, deprecated and optional selectors, weak delegate, where background errors arrive | §3.2, §3.3 |
| A lost key can't be rescued by Developer ID with our settings; key rotation needs the bridge release kept in the feed | §6.2 |
| Secret handling: its own job, on stdin, in a protected environment | §6.1 |
| "Quit and Install" must list running programs itself | §3.4 |
| Silent updates and pins before 1.0; the cask caveat for users with the updater off | §2 |
| Build numbers read from the zip and checked to go up | §6.1 |
| End-to-end test: the right variable name, local HTTP instead of a trusted certificate, install only after quit, pins during the install automated | §8 |
| PR plan: a first PR for fetching Sparkle; clashes with other work on term-app; missing items | §9 |
| Smaller: zap paths, network per check, thinning Sparkle, the version to pin, objc2 details, log size | throughout |

---

## 1. Decisions at a glance

| Question | Decision | Why |
|---|---|---|
| Which updater | Sparkle 2, **2.10.1 or later** (pinned, MIT), embedded as `Contents/Frameworks/Sparkle.framework`, thinned to arm64 | the standard for apps outside the App Store; EdDSA-signed updates and feeds; installs over a running app |
| How it is loaded | **at run time**, with `NSBundle` `load`, only in the app's primary process (`Role::App`); the library does not link Sparkle | pins load the same library and must never load Sparkle; the bare binary has no Sparkle |
| Who decides | `update::decide(&Context) -> Decision`, a plain function: off for pins, the bare binary, a second app process, dogfood builds, nightlies, ad-hoc builds, `update = false`, a missing key or framework | testable without Sparkle; every "off" has a reason in the log |
| The driver | our own `SPUUserDriver`: for background checks it never shows anything and always **dismisses**, so a downloaded update installs **when the app quits**; checks the user asked for get standard alerts | a terminal can't relaunch itself under running programs |
| When updates install | on quit (Cmd+Q, or closing the last window, which quits the app) | Sparkle's install-on-quit; nothing is interrupted |
| Daily check | **Sparkle's own scheduler** (`SUEnableAutomaticChecks`, `SUScheduledCheckInterval = 86400`): a one-shot wall-clock timer, re-armed after each check, counted from the last check across launches | 0 wakeups between checks; no second record of "last check" |
| System profiling | off | roadmap |
| Feed | one appcast with the newest releases, an asset of each tagged release: `https://github.com/0xdeafcafe/terminal-decline/releases/latest/download/appcast.xml`; signed (`SURequireSignedFeed`) | a stable URL with no server of our own; `latest` skips prereleases, so nightlies never reach users; open question 5 |
| Who may be updated | only notarized builds signed with our Developer ID; the release job refuses to publish an appcast otherwise | Sparkle trusts the EdDSA key alone; an ad-hoc release would reset every user's permissions and break every release pin |
| Signing updates | EdDSA (`sign_update`) in a separate macOS job, the key on stdin from `SPARKLE_ED_PRIVATE_KEY` in a protected environment; the public key committed (`packaging/sparkle-public-key`) and written into Info.plist as `SUPublicEDKey` | the key never touches disk or the build steps |
| Version order | `sparkle:version` = `CFBundleVersion`, the integer build number that only goes up (N.9), read from the zip and checked to be above the current latest | Sparkle compares it; a wrong or lower number would loop or stop updates |
| A check the user asks for | Terminal Decline › Check for Updates…; answers are standard alerts | menus and alerts are macOS's own UI, allowed by the zero-UI rule; open question 2 |

---

## 2. Where it runs, and where it doesn't

```rust
pub struct Context<'a> {
    pub role: Role,                    // App, Pin, Bare (pins.md §2.5)
    pub primary: bool,                 // this process holds the updater lock
    pub bundle_id: Option<&'a str>,    // NSBundle.mainBundle's
    pub team_id: Option<&'a str>,      // from our own code signature; None when ad-hoc
    pub dogfood: bool,                 // Contents/Resources/dogfood exists (reload.rs)
    pub version: &'a str,              // CFBundleShortVersionString
    pub setting: bool,                 // `update = true` (default)
    pub public_key: bool,              // SUPublicEDKey present and 32 bytes of base64
    pub framework: bool,               // Contents/Frameworks/Sparkle.framework exists
    pub test_feed: bool,               // the test-only feed override is set (§8)
}

pub enum Decision { On, Off(&'static str) }
```

`decide` returns `Off` with the reason for: a pin or the bare binary; a
second app process (below); a bundle id other than
`red.forbes.terminal-decline`; a dogfood build (it would replace Alex's
own build with a release); a version containing `-nightly` or `-dev`; an
ad-hoc build (no team ID), unless the test feed is set; `update = false`;
no public key; no framework. With the test control enabled
(`DECLINE_TEST_DUMP`), it is off unless the test feed is set, so smoke and
packaging tests never reach GitHub. The reason is logged once at launch
to `update.log` in the logs directory (capped at 256 KB, oldest half
dropped) and to os_log; nothing is shown.

What Sparkle checks itself (a translocated app, a read-only volume, a
folder it can't write) needs no input here: it refuses quietly in the
background, and with §3.2's rule a background check never asks for a
password.

**One updater per user.** `terminal-decline -e …` (or `decline`) run from
a shell starts a second full app process from the same bundle as
`Role::App`. Two updaters would share Sparkle's helper jobs, and the
helper waits only on the first running copy it finds, so an update could
install under the other one. The first app process takes an exclusive
`flock` on `~/Library/Caches/terminal-decline/updater.lock` for its whole
life (no wakeups); a process that can't is `primary = false`. Such a
process keeps running the old code after an update, like pins.

The check is a pure function and table-tested; the inputs are gathered by
a few lines of AppKit code at launch.

**Pins.** A pin runs the same library (pins.md), as `Role::Pin`, so
`decide` is `Off` and Sparkle is never loaded. When the app updates,
running pins keep the old library mapped and pick up the new one next
time they start, through the frozen interface (pins.md §3.5). The first
app launch after an update checks the pins' status (`pin --list`'s
logic) and shows pins.md §5.4's notice if any can't use the new library.

**Homebrew.** The cask installs into `/Applications` as the user, so the
app is writable and Sparkle replaces it in place. `brew upgrade` skips
casks with `auto_updates true` unless `--greedy`, so users whose updater is
off (an app owned by root after a `sudo` install, `update = false`) get no
updates from either; the cask caveat says `brew upgrade --greedy
terminal-decline`. Brew compares its own recorded version, so `--greedy`
with a tap that lags behind can install an older version over a
self-updated app; the caveat says so. The cask's `binary` symlinks point
into the bundle, which keeps its path.

---

## 3. Sparkle in the process

### 3.1 Loading

At `applicationDidFinishLaunching`, if `decide` is `On`, on the main
thread:

1. `NSBundle bundleWithPath:` of `Contents/Frameworks/Sparkle.framework`,
   `loadAndReturnError:`. A failure (missing, or library validation
   refusing a framework from another team) is logged and the updater
   stays off.
2. Only then are our driver and delegate classes registered (objc2
   registers a class on first use, so they are first used after the
   load), declaring conformance to `SPUUserDriver` and
   `SPUUpdaterDelegate` at run time (`objc_getProtocol`).
3. `SPUUpdater initWithHostBundle:mainBundle applicationBundle:mainBundle
   userDriver:driver delegate:delegate`, then `startUpdater:`. The app
   delegate keeps strong references to the updater, the driver and the
   delegate (Sparkle holds its delegate weakly).

Loading happens once; Objective-C frameworks can't be unloaded. Its
memory is measured (§7).

### 3.2 The silent driver

Replies follow Sparkle's own state: `state.userInitiated`, `state.stage`
(not downloaded, downloaded, installing) and `appcastItem.informationOnlyUpdate`.
Background checks never show anything.

| Call | Background (`userInitiated` false) | User asked |
|---|---|---|
| `showUpdatePermissionRequest:reply:` | not expected (`SUEnableAutomaticChecks` is in Info.plist); if it comes: automatic checks = our setting, system profile no | — |
| `showUserInitiatedUpdateCheckWithCancellation:` | — | nothing until the answer |
| `showUpdateFoundWithAppcastItem:state:reply:` | **dismiss**, and log why (critical, information-only, major upgrade, needs a password, or an update that has waited). With a normal update Sparkle downloads and stages it without calling this at all. Dismissing at the installing stage still installs on quit | not downloaded or downloaded: **install** (download now; the user is asked once, at the next row); installing: the "Quit and Install" alert (§3.4); information-only: an alert with a link to the release page |
| `showUpdateReleaseNotesWithDownloadData:` / `…FailedToDownloadWithError:` | ignored (no release notes link in the feed) | ignored |
| `showUpdateNotFoundWithError:acknowledgement:` | — (Sparkle doesn't call it for background checks) | alert "You're up to date" |
| `showUpdaterError:acknowledgement:` | — (background errors arrive through the delegate, §3.3) | alert with the error |
| download and extraction progress calls | nothing | nothing |
| `showReadyToInstallAndRelaunch:` | **dismiss** (installs on quit) | the "Quit and Install" alert |
| `showInstallingUpdateWithApplicationTerminated:retryTerminatingApplication:` | nothing | nothing |
| `showUpdateInstalledAndRelaunched:acknowledgement:` | acknowledge | acknowledge |
| `showUpdateInFocus` (optional since 2.8), `dismissUpdateInstallation` | nothing | close our alert if one is open |

The table is data (`fn answer(call, &State) -> Answer`), tested as a
table; the Objective-C methods only forward to it, copying Sparkle's reply
blocks (`RcBlock`) when they answer after an alert. Sparkle sends required
methods without checking that they exist, so a missing one is an
Objective-C exception: a test lists every required method of the pinned
Sparkle's protocol (`protocol_copyMethodDescriptionList`) and checks our
class answers each.

### 3.3 The delegate

- `updater:mayPerformUpdateCheck:error:` → `decide(…)` again (the setting
  may have changed through live reload); this is also how `update =
  false` stops Sparkle's scheduled checks.
- `feedURLStringForUpdater:` → Info.plist's `SUFeedURL`, or the test
  feed (`TD_UPDATE_FEED` from the environment, honoured **only** with
  `DECLINE_TEST_DUMP` set).
- `allowedChannelsForUpdater:` → none (stable only; open question 3).
- `updater:willInstallUpdateOnQuit:immediateInstallationBlock:` → NO
  (keep waiting for quit, and let later checks continue); it is also the
  "ready" signal written to `update.log` and to the test dump.
- `updater:didAbortWithError:` and Sparkle's "update check finished"
  delegate call (named per the pinned version's header) → one line in `update.log`; background failures are only seen here.

### 3.4 Check for Updates…, and Quit and Install

A menu item in the app menu, under About, only when `decide` is `On`. It
calls `checkForUpdates` (if `sessionInProgress` is false; otherwise it
brings the existing alert forward). It is also a keybind action
(`check_for_updates`, unbound by default).

**Quit and Install**: Sparkle quits the app through a terminate request.
`applicationShouldTerminate:` asks about running programs only with
`confirm_close = true`, which is off by default, so the alert itself lists
running programs, with the question the app already asks on quit
(`running::quit_question`): "Quit and Install will end top and 2 other
programs." [Quit and Install] [Later]. The app doesn't relaunch afterwards
(open question 2 asks whether it should, with windows reopened).

### 3.5 Installing, and what the signature proves

- The update is a zip of the notarized, stapled app (`ditto -c -k
  --keepParent`, which keeps symlinks and signatures).
- Sparkle checks the zip's EdDSA signature before extracting it
  (`SUVerifyUpdateBeforeExtraction`) and then only that the extracted app
  has *a* valid code signature. **It accepts any code signature when the
  EdDSA keys match**, ad-hoc included. So the EdDSA private key alone
  authorises code on every installed copy, and our safety rests on the
  release job publishing only Developer ID, notarized builds (§6.1).
- On quit, Sparkle's `Autoupdate` helper replaces the bundle at its path.
  With `Autoupdate` signed by the same team as the app, it swaps the two
  bundles in one step; otherwise it moves the old one out and the new one
  in, and a pin starting in between would find no app. A packaging test
  checks the team matches.
- A user who never quits: the update waits, downloaded. After
  `SUScheduledImpatientCheckInterval` (Sparkle's default one week)
  Sparkle calls the driver again; the background rule dismisses and logs
  it. Open question 1 asks whether that moment should show an alert.
- Windows aren't restored after an update (the user quit). The dogfood
  restart (reload.rs) is separate and unchanged.

---

## 4. The daily check

Sparkle's own scheduler: `SUEnableAutomaticChecks = true`,
`SUScheduledCheckInterval = 86400`. It is a one-shot wall-clock dispatch
timer, armed for the time left until a day after `SULastCheckTime` (kept
across launches, so a Mac that quits the app daily still checks daily),
re-armed after each check. Between checks it causes no wakeups; `top
-stats idlew` must still read 0 over 5 s at any time but the minute of a
check (measured in U-4). `update = false` is applied through
`updater:mayPerformUpdateCheck:error:`.

DESIGN.md's goal becomes (in U-1): "No timers, no redraws, no wakeups
while nothing changes. Cursor blink (when an app asks for it) is the only
timer of our own, and it stops when the window is hidden or unfocused.
Sparkle keeps one daily wall-clock timer for the update check."

`NSBackgroundActivityScheduler` was the other choice (macOS defers it for
power and network); it saves no wakeups and would need its own launch
check and record. Open question 7.

A check is three HTTPS requests (two redirects, then the asset host),
two TLS handshakes, about 10 KB; an update is one download of the zip
(~3–5 MB).

---

## 5. Packaging

### 5.1 Getting Sparkle

`scripts/sparkle.sh` downloads the pinned release (`Sparkle-2.x.y.tar.xz`
from Sparkle's GitHub releases) into `target/sparkle/<version>/`, checks
its SHA-256 against the value in the script, and unpacks
`Sparkle.framework` and `bin/sign_update`. Nothing is committed. macOS CI
runs it too (for U-1's protocol test). Sparkle's licence (MIT, with the
ed25519 and bsdiff licences it bundles) goes into the third-party notices.

### 5.2 Info.plist keys (bundle.sh)

| Key | Value |
|---|---|
| `SUFeedURL` | `https://github.com/0xdeafcafe/terminal-decline/releases/latest/download/appcast.xml` |
| `SUPublicEDKey` | contents of `packaging/sparkle-public-key` (omitted until it exists; the updater is then off) |
| `SUEnableAutomaticChecks` | true |
| `SUScheduledCheckInterval` | 86400 |
| `SUAutomaticallyUpdate` | true |
| `SUAllowsAutomaticUpdates` | true |
| `SUEnableSystemProfiling` | false |
| `SUVerifyUpdateBeforeExtraction` | true (required by signed feeds) |
| `SURequireSignedFeed` | true (exists since 2.9; the release job signs the appcast) |
| `SUScheduledImpatientCheckInterval` | per open question 1 (default one week) |

Test builds (§8) get extra switches in bundle.sh: a test public key,
`NSAppTransportSecurity`/`NSAllowsLocalNetworking = true`, and a short
impatient interval. Release builds never have them; the release job
checks.

### 5.3 In the bundle, and signing

```
Contents/Frameworks/Sparkle.framework/
  Versions/B/Sparkle            the framework's library
  Versions/B/Autoupdate         installs on quit
  Versions/B/Updater.app        Sparkle's helper app: required (it waits for the app to quit)
  Versions/B/XPCServices/       removed, with the top-level XPCServices symlink (only sandboxed apps need them)
```

Before signing: `lipo -thin arm64` on each binary, unused localisations
removed. Signed inside-out in bundle.sh, after the library and before the
stub and the app (pins.md §2.4), each with our identity, `--options
runtime --timestamp`:

```sh
fw="Contents/Frameworks/Sparkle.framework"
codesign -f -s "$ID" $opts "$fw/Versions/B/Autoupdate"
codesign -f -s "$ID" $opts "$fw/Versions/B/Updater.app"
codesign -f -s "$ID" $opts "$fw"
```

Removing only the `Versions/B/XPCServices` folder would leave the
top-level symlink pointing at nothing and fail `codesign --verify
--strict`; both go.

### 5.4 The cask

`auto_updates true` stays; the caveat gains the `--greedy` note (§2);
`zap` gains `~/Library/Caches/red.forbes.terminal-decline` (Sparkle keeps
its files under `…/org.sparkle-project.Sparkle` there) and
`~/Library/HTTPStorages/red.forbes.terminal-decline`.

---

## 6. The appcast in the release workflow

### 6.1 Steps

A separate macOS job `sign` (needs `build`), only for tags, in a GitHub
Environment `release` limited to `v*` tags. The build job runs `cargo
build` (every dependency's build script) and third-party actions; the key
must never be in its environment.

1. Download the build artifact and the pinned Sparkle (`sparkle.sh`).
2. **Refuse ad-hoc or unnotarized builds**: `codesign -dv` shows
   `TeamIdentifier` = `APPLE_TEAM_ID`, `spctl -a -t exec` passes, `xcrun
   stapler validate` passes; otherwise the job fails.
3. **Check the keys**: derive the public key from the secret (base64 of
   the 32-byte Ed25519 private key; Python's
   `Ed25519PrivateKey.from_private_bytes`) and compare it byte for byte
   with `packaging/sparkle-public-key`, with `SUPublicEDKey` read back
   from the built app (`plutil -extract`), and with `SUPublicEDKey` of the
   current `latest` release, the key users actually have. Any mismatch
   fails the job. (`sign_update`'s verify mode checks against the key it
   is given, so it can't catch this.)
4. **Read the versions from the zip**: `appcast.py` takes
   `CFBundleVersion`, `CFBundleShortVersionString` and
   `LSMinimumSystemVersion` from the zipped app's Info.plist, and fails
   unless the build number is greater than the newest in the current
   `latest` appcast.
5. **Sign the zip**: `echo "$KEY" | sign_update --ed-key-file - <zip>`
   (the key on stdin, never on disk; the secret is given to this step
   only) → `sparkle:edSignature` and `length`.
6. **Write the appcast**: the new item plus the last few items carried
   forward from the current `latest` appcast:

```xml
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Terminal Decline</title>
    <item>
      <title>0.4.0</title>
      <pubDate>Thu, 29 Oct 2026 03:00:00 +0000</pubDate>
      <sparkle:version>1234</sparkle:version>
      <sparkle:shortVersionString>0.4.0</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
      <enclosure url="https://github.com/0xdeafcafe/terminal-decline/releases/download/v0.4.0/Terminal-Decline-0.4.0-macos-arm64.zip"
                 length="3456789" type="application/octet-stream"
                 sparkle:edSignature="…"/>
    </item>
  </channel>
</rss>
```

7. **Sign the appcast** (`sign_update appcast.xml` embeds the feed
   signature, for `SURequireSignedFeed`).
8. Upload `appcast.xml` with the zip as the job's artifact; the `release`
   job checks the file is there before `gh release create`, so
   `releases/latest/download/appcast.xml` is always the newest release's.

Once the first self-updating release is out, a tag build without
`SPARKLE_ED_PRIVATE_KEY` **fails** (a release without an appcast would
become `latest` and stop all updates). Before that it is a warning. There
are no delta updates (the zip is a few MB). `appcast.py --self-test` runs
in CI's fmt job like the other scripts: fixed inputs, parse back, every
field, XML escaping, carrying items forward.

### 6.2 The keys (needs Alex)

- Alex runs Sparkle's `generate_keys` once, exports the private key
  (`generate_keys -x`), stores it as `SPARKLE_ED_PRIVATE_KEY` in the
  `release` environment, keeps a backup in his password manager and one
  offline, and commits the public key (`generate_keys -p`) as
  `packaging/sparkle-public-key`. Whether the key also stays in his login
  keychain is open question 8.
- **Losing the key.** With `SUVerifyUpdateBeforeExtraction` on and zip
  updates, Sparkle's way to bring in a new key with a same-team Developer
  ID release can't work (it checks the archive's code signature, and a
  zip has none). So a lost key means no installed copy can be updated
  again, and users reinstall by hand. Turning both
  `SUVerifyUpdateBeforeExtraction` and signed feeds off would allow that
  rescue, at the cost of the feed signature. Open question 4.
- **Rotating the key**: a bridge release signed with the old key that
  carries the new public key; the bridge stays in the feed, and the next
  item has `sparkle:minimumUpdateVersion` = the bridge's build (Sparkle
  2.9+), so a copy that missed the bridge updates to it first instead of
  being stranded.

### 6.3 Nightlies and private builds

Nightlies are prereleases, so `releases/latest` never points at them, and
`decide` turns the updater off in builds whose version says `-nightly`.
While the repository is private (until 0.1.0, E.16), release assets need
authentication, so a check fails and logs the HTTP error. Updates also
wait for the Developer ID secrets (N.4), since nothing ad-hoc is
published.

**Landed (U-3, with U-2's remaining parts).** What was built, and where
it differs from the above:
- **`scripts/appcast.py`** (Python's standard library only): `check-release`,
  `public-key`, `newest`, `verify`, `verify-feed`, `write` and
  `--self-test` (in CI's fmt job). It verifies Ed25519 signatures itself
  (RFC 8032, checked against its test vectors and against `sign_update`'s
  output), so no package is installed in the job that holds the key.
- **The key comparison works the other way round from step 3**: the
  secret never reaches Python. `sign_update` signs the zip with the key on
  stdin, and `appcast.py verify` checks that signature against
  `packaging/sparkle-public-key`; `check-release` checks the app's
  `SUPublicEDKey` is that key, and the `sign` job reads `SUPublicEDKey`
  from the zip the current appcast's newest item points at. A wrong
  secret, a wrong committed key or a key users don't have all fail the
  job before anything is published. The embedded feed signature is
  checked against the committed key too (`verify-feed`).
- **Without the Apple secrets** (or without `SPARKLE_ED_PRIVATE_KEY`) the
  `sign` job doesn't fail: it lists the missing pieces in the job summary
  and as warnings, and the release goes out without an appcast. Once the
  latest release has an appcast, the same case fails the job, and the
  release isn't made. The build job reports `signing` and `notarized`;
  the `sign` job checks them again on the app (`TeamIdentifier` of the
  app and of Sparkle's `Autoupdate` equal to `APPLE_TEAM_ID`, `spctl`
  saying "Notarized Developer ID", `stapler validate`).
- **Release checks on every tag** (`check-release`, whether or not updates
  are published): bundle id, the tag's version, an integer build number,
  the committed public key, the release feed URL, signed feeds and
  verification before extraction on, profiling off, the one-week
  impatient interval, no `NSAppTransportSecurity`, no dogfood marker,
  Sparkle embedded.
- **Carried forward**: up to 5 items in all, newest first, older items
  copied as they were (a bridge's `sparkle:minimumUpdateVersion`
  included). A bridge older than the 5 newest would drop out; keep it in
  mind when rotating the key.
- **U-2's rest**: the cask's caveat explains `brew upgrade --greedy` and
  that it can install an older version than the app updated itself to;
  `zap` gains `~/Library/Caches/red.forbes.terminal-decline` and
  `~/Library/HTTPStorages/red.forbes.terminal-decline`.
  `THIRD-PARTY-NOTICES.md` carries Sparkle's licence (with bsdiff,
  sais-lite, ed25519 and SUSignatureVerifier), and bundle.sh copies it to
  `Contents/Resources/` and refuses a Sparkle version it has no section
  for.
- **Still needs Alex**: the `release` environment limited to `v*` tags,
  with `SPARKLE_ED_PRIVATE_KEY` moved into it (as a repository secret it
  is readable by any job); and the Apple secrets (N.4).

---

## 7. Costs, and how to measure them

| What | Expected | How measured |
|---|---|---|
| App bundle and zip | + ~2–3 MB unpacked, + ~1 MB zipped (Sparkle thinned, without XPC services and extra localisations) | `du -sk` of the bundle, zip size in the release job summary, before/after |
| Footprint, idle, updater on | + 1–2 MB once Sparkle is loaded (classes, CFNetwork and TLS state after a check) | `/usr/bin/footprint -p <pid>` with `update = true` vs `false` on the standard set, at idle and 10 minutes after a check (CFNetwork state stays loaded). Once loaded, Sparkle stays for the life of the process |
| Helper processes while an update waits | `Autoupdate` and `Updater.app` (an AppKit app) run as launchd jobs from extraction until the app quits, which may be weeks | `footprint` and `top -stats idlew` of both after an update is staged; both must be 0 wakeups |
| Footprint in pins | + 0 | pins never load Sparkle: `vmmap <pid> | grep Sparkle` empty (a smoke test) |
| Wakeups | 0 between checks | `top -l 2 -s 5 -stats cpu,idlew` at idle with the updater on, and 2 and 10 minutes after a check |
| Launch time | + ≤ 5 ms (framework load after the first window) | pins.md §7's launch script on the app, updater on vs off |
| Network | three requests, ~10 KB, per day; one zip per release | `update.log` |

---

## 8. Tests

- **`decide`**: a table with one row per input that turns it off and one
  that leaves it on (role, primary, bundle id, team, dogfood, nightly and
  dev versions, setting, key, framework, test mode with and without the
  test feed).
- **The driver's answers**: §3.2's table as data, tested row by row,
  including each stage and information-only items.
- **Protocol coverage**: with Sparkle fetched by `sparkle.sh` in macOS CI,
  every required `SPUUserDriver` method and every delegate method we rely
  on exists on our classes and in Sparkle.
- **One updater**: two app processes from one bundle: only the first
  takes the lock.
- **Pins never load Sparkle**: start a pin, `vmmap` shows no Sparkle
  image; the app with the updater on does.
- **Packaging**: the bundle has `Updater.app`, no `XPCServices` (folder or
  symlink), arm64 only; `Autoupdate`'s team equals the app's; release
  builds have none of the test switches.
- **appcast.py**: `--self-test` in CI.
- **End to end** (`scripts/update-e2e.sh`; run by hand on macOS and in the
  weekly workflow, whose runner must have a GUI session for Sparkle's
  launchd jobs — checked on its first run; no secrets needed):
  1. generate a throwaway key pair;
  2. build version A (build N) and version B (build N + 1), both ad-hoc,
     with the test switches (throwaway public key, local networking, a
     60 s impatient interval), B signed with the throwaway private key;
  3. serve B's zip and an appcast over `http://127.0.0.1` (Sparkle only
     warns about an http feed; local networking satisfies App Transport
     Security);
  4. start A with `DECLINE_TEST_DUMP` and `TD_UPDATE_FEED` set, and a pin
     made from A running; send the control command `update-check` (a
     background check); wait for the dump's `update` field to read
     `ready`;
  5. check the bundle on disk is **still** build N while A runs, then quit
     A through the control;
  6. check the bundle is now build N + 1, `codesign --verify --strict
     --deep` passes, the pin started again runs B's library, and no window
     other than ours appeared during the run (`CGWindowListCopyWindowInfo`
     sampled: no Sparkle window, no password dialog);
  7. a critical item, and an update left waiting past the 60 s impatient
     interval: A is still running afterwards and no window appeared;
  8. tamper: one byte of the zip changed → A stays at N, `update.log`
     names the signature failure;
  9. downgrade: an appcast offering build N − 1 → nothing happens.

  (An update signed by another team is accepted by Sparkle when the EdDSA
  signature is right, §3.5, so it isn't a test here; the release job's
  checks are what stop it.)

---

## 9. PR plan

B.4 comes after phase P (pins.md), which rewrites `bundle.sh` and adds the
`Role` the updater needs. term-app has other work queued at the same
time (B.1 ligatures, B.2 splits' menu actions and control commands, I.5
images); U-1 is sequenced after those PRs that edit `keybind` and
`control`, or rebased onto them.

| # | Module | PR | Size | After |
|---|---|---|---|---|
| U-0 | packaging + ci | `scripts/sparkle.sh` (pinned version and hash), its run in macOS CI and before bundle.sh in release.yml | S | P-4 |
| U-1 | app | `update` module: `decide` with the lock, run-time loading, driver and delegate classes, the answer table, the `update` setting (in E.18's schema), Check for Updates… with Quit and Install, its keybind action, `update.log`, the `update-check` control command and `update` dump field (test-only), the post-update pin check, DESIGN.md's goal | L | U-0, P-5 |
| U-2 | packaging | bundle.sh embeds Sparkle (thinned, XPC services and symlink removed), signs its parts, writes the Info.plist keys and the test switches; `packaging/sparkle-public-key` once Alex has made it; cask caveat and `zap`; third-party notices | S | U-0 |
| U-3 | ci | release.yml: the `sign` job in the `release` environment (team/notarization checks, key comparison, version checks, signing on stdin, appcast and its signature, items carried forward), the artifact and the `release` job's check; `appcast.py` and its self-test | M | U-0 (parallel with U-1, U-2) |
| U-4 | test + docs | `scripts/update-e2e.sh` and its weekly run; DESIGN.md "Updates" section; numbers of §7 | S | U-1, U-2, U-3 |

U-1, U-2 and U-3 run at once after U-0 (term-app, bundle.sh, release.yml).

---

## 10. Open questions (need Alex)

Decided 29 Oct 2026: 1 a Quit and Install alert after 7 days waiting;
3 stable only in 1.0; 5 the feed is GitHub's `releases/latest`, for good;
9 updates only for Developer ID, notarized builds; the rest as the design
picks. Alex generates the EdDSA key pair (secret plus two backups).

1. **Users who never quit.** Updates install on quit, and a terminal can
   run for weeks. After an update has waited N days (Sparkle's
   `SUScheduledImpatientCheckInterval`, default 7), show a standard alert
   offering "Quit and Install", or stay silent forever? What N?
2. **Quit and Install.** Keep the menu item and alert at all? If so, just
   quit, or relaunch with windows reopened in the same folders, as the
   dogfood restart does?
3. **Channels.** Stable only in 1.0, or also a beta/nightly channel you opt
   into with a setting (needs a second appcast)?
4. **The key and its rescue.** Keep `SUVerifyUpdateBeforeExtraction` and
   signed feeds, accepting that a lost key can never be replaced (users
   would reinstall), or turn both off so a Developer ID release could bring
   in a new key?
5. **Feed URL.** It is fixed in every installed copy for good, and the
   repository has been renamed once already. GitHub's `releases/latest`
   permanently, or a URL you control (your own domain, or Pages with a
   custom domain) that can redirect anywhere later?
6. **Apps that need an administrator password** to update (installed by
   another user or with `sudo`): stay silent and log, or let Check for
   Updates… ask for the password through macOS's own dialog?
7. **Daily check.** Sparkle's own daily timer (the design's choice; the
   DESIGN.md goal says "Sparkle keeps one daily wall-clock timer"), or
   `NSBackgroundActivityScheduler`, which macOS defers for power and
   network?
8. **The private key.** After exporting it, keep it in your login keychain
   too, or only the repository secret and the two backups?
9. **Ad-hoc builds.** The design publishes updates only for Developer ID,
   notarized builds, so the updater does nothing until N.4's secrets
   exist. Agreed?
10. **Telling the user.** Say nothing after an update, or print one dim
    line in the first window after it ("updated to 0.5.0"), as config
    notices are shown?
