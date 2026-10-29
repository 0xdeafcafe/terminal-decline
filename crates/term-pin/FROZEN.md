# What a pin's stub freezes

Every pin carries a copy of the stub (`src/bin/pin-stub.rs`) made when the
pin was made, and keeps it: changing the copy would change the pin's code
signature, and macOS would ask again for every permission the pin has.
So from the first public build that makes stub pins (0.1.0;
docs/design/pins.md §3.5, §11) everything below stays as it is. A change
means a new interface version (a new struct and function in
`abi/terminal_decline_pin.h`, the library exporting both) or a new key,
never an edit of what's here.

- **The interface**: `abi/terminal_decline_pin.h`, `struct
  td_pin_info_v1` and `td_pin_main_v1`. The library exports every version
  ever in a public build. A stub of version N tries `td_pin_main_vN` down
  to 1 (`TDPinAbi` caps N).
- **Info.plist keys the stub reads**: `TDPinAbi` (integer ≥ 1), `TDPinId`,
  `TDAppBundleId`, `TDLibraryRequirement` (strings),
  `CFBundleShortVersionString` (passed as `stub_version`), `CFBundleName`
  (for messages).
- **The search order** (`src/find.rs`): the override file
  `$HOME/.config/terminal-decline/pin-app` (`HOME`, else the password
  database; `XDG_CONFIG_HOME` ignored; its first line, an absolute path,
  made real; anything else refused), then `/Applications/Terminal
  Decline.app`, then `$HOME/Applications/Terminal Decline.app`, then
  LaunchServices' apps for `TDAppBundleId`, skipping any in a Trash, under
  `/Volumes`, translocated (`/AppTranslocation/`) or in `~/Downloads`,
  highest `CFBundleVersion` first. A candidate must have that bundle id
  and `Contents/Frameworks/libterminal_decline.dylib`. The first that
  passes the check, loads and has an entry point wins.
- **The check** (`src/sec.rs`): `SecStaticCodeCheckValidityWithErrors`
  with `kSecCSStrictValidate | kSecCSNoNetworkAccess` against
  `TDLibraryRequirement`. The forms written into pins (`src/abi.rs`): dev
  `identifier "red.forbes.terminal-decline.lib"`; release `anchor apple
  generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and
  certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate
  leaf[subject.OU] = "<TEAM>" and identifier "red.forbes.terminal-decline.lib"`.
- **Loading**: `dlopen(path, RTLD_NOW | RTLD_LOCAL)` of the real path.
- **Linked strongly**: CoreFoundation, Security, libSystem, libiconv (Rust's
  std). LaunchServices only through `dlsym`, so its removal skips a step
  instead of stopping the stub.
- **Messages** (`src/message.rs`): the alert title "<name> can't start",
  the five texts, the buttons "Get Terminal Decline" and "Quit", the
  download URL `https://github.com/0xdeafcafe/terminal-decline/releases`.
  Shown with `CFUserNotificationDisplayAlert` (no AppKit) and on stderr;
  exit status 1. `TD_PIN_NO_ALERT` (tests) skips the alert.
- **How it is built**: the `release-stub` profile (panic abort, size
  optimised, fat LTO, stripped).

The launcher (`src/bin/launcher.rs`, `td_app_main_v1`) ships with the
library in the same app, so none of this applies to it.

Tests: `tests/stub.rs` with `TD_PIN_TEST_STUB=<a stub binary>` runs any
stub (an earlier release's) against today's checks;
`TD_PIN_TEST_IDENTITY="Developer ID Application: … (TEAM)"` adds the
release requirement with a real certificate.
