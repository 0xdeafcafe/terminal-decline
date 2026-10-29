//! The app bundle `scripts/bundle.sh` makes (docs/design/pins.md §2): the
//! launcher, the library and the pin stub in their places, signed inside
//! out; an Info.plist that is valid and declares what the app answers
//! (folders opened with it, the Services menu items, named with
//! [`terminal_decline_app::PRODUCT_NAME`]) and the usage strings; and an
//! app that starts.
#![cfg(target_os = "macos")]

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

/// `plutil -extract KEY raw` of `plist`.
fn extract(plist: &Path, key: &str) -> String {
    let out = Command::new("/usr/bin/plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(plist)
        .output()
        .unwrap();
    assert!(out.status.success(), "{key}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

struct OutDir(PathBuf);

impl OutDir {
    fn new(name: &str) -> OutDir {
        let dir = std::env::temp_dir().join(format!("terminal-decline-bundle-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        OutDir(dir)
    }
}

impl Drop for OutDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `codesign -d` of `path`, with `extra` arguments: what it prints.
fn codesign_display(path: &Path, extra: &[&str]) -> String {
    let out = Command::new("/usr/bin/codesign")
        .arg("-d")
        .args(extra)
        .arg("-vvv")
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "codesign -d {}: {}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    // codesign prints the details on stderr, the entitlements on stdout.
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

fn run_ok(cmd: &mut Command) -> String {
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{cmd:?}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn info_plist_declares_folders_services_and_usage() {
    let out = OutDir::new("plist");
    let app = support::bundle(&out.0, &[]);
    let plist = app.join("Contents/Info.plist");
    let lint = Command::new("/usr/bin/plutil").arg("-lint").arg(&plist).output().unwrap();
    assert!(lint.status.success(), "{}", String::from_utf8_lossy(&lint.stdout));

    assert_eq!(extract(&plist, "CFBundleIdentifier"), terminal_decline_app::BUNDLE_ID);
    assert_eq!(extract(&plist, "CFBundleExecutable"), "terminal-decline");
    assert_eq!(extract(&plist, "CFBundleShortVersionString"), "1.2.3");
    assert_eq!(extract(&plist, "CFBundleVersion"), "7");
    assert_eq!(extract(&plist, "CFBundleDocumentTypes.0.LSItemContentTypes.0"), "public.folder");
    let bundle_name = extract(&plist, "CFBundleName");
    for (i, (message, title)) in [("newWindowAtFolder", "Window"), ("newTabAtFolder", "Tab")].into_iter().enumerate() {
        let service = |key: &str| extract(&plist, &format!("NSServices.{i}.{key}"));
        // The app's delegate answers `<message>:userData:error:`.
        assert_eq!(service("NSMessage"), message);
        assert_eq!(
            service("NSMenuItem.default"),
            format!("New {} {title} at Folder", terminal_decline_app::PRODUCT_NAME)
        );
        assert_eq!(service("NSPortName"), bundle_name);
        assert_eq!(service("NSSendFileTypes.0"), "public.folder");
    }
    // pins.md §2.3: every kind of access a program in the terminal might
    // ask for.
    for key in [
        "NSMicrophoneUsageDescription",
        "NSCameraUsageDescription",
        "NSAudioCaptureUsageDescription",
        "NSAppleEventsUsageDescription",
        "NSAppleMusicUsageDescription",
        "NSLocationUsageDescription",
        "NSLocationWhenInUseUsageDescription",
        "NSContactsUsageDescription",
        "NSCalendarsUsageDescription",
        "NSCalendarsFullAccessUsageDescription",
        "NSRemindersUsageDescription",
        "NSRemindersFullAccessUsageDescription",
        "NSPhotoLibraryUsageDescription",
        "NSBluetoothAlwaysUsageDescription",
        "NSLocalNetworkUsageDescription",
        "NSSpeechRecognitionUsageDescription",
        "NSDesktopFolderUsageDescription",
        "NSDocumentsFolderUsageDescription",
        "NSDownloadsFolderUsageDescription",
        "NSRemovableVolumesUsageDescription",
        "NSNetworkVolumesUsageDescription",
        "NSSystemAdministrationUsageDescription",
    ] {
        let text = extract(&plist, key);
        let start = format!("A program running in {} wants to ", terminal_decline_app::PRODUCT_NAME);
        assert!(text.starts_with(&start) && text.ends_with('.'), "{key}: {text}");
    }
}

/// pins.md §2.1 and §2.4: the parts in their places, each signed with its
/// own identifier, the whole verifying strictly; the app starts.
#[test]
fn layout_signing_and_the_app_starts() {
    let out = OutDir::new("layout");
    let app = support::bundle(&out.0, &[]);
    let contents = app.join("Contents");
    let lib = contents.join("Frameworks/libterminal_decline.dylib");
    let stub = contents.join("Helpers/terminal-decline-pin");
    let launcher = support::launcher(&app);
    for p in [&lib, &stub, &launcher] {
        assert!(p.is_file(), "{} missing", p.display());
    }
    assert!(!contents.join("Resources/dogfood").exists());

    assert!(codesign_display(&lib, &[]).contains("Identifier=red.forbes.terminal-decline.lib"));
    assert!(codesign_display(&stub, &[]).contains("Identifier=red.forbes.terminal-decline.pin-stub"));
    let main = codesign_display(&app, &["--entitlements", "-"]);
    assert!(main.contains("Identifier=red.forbes.terminal-decline\n"), "{main}");
    assert!(main.contains("com.apple.security.device.camera"), "{main}");
    assert!(!main.contains("disable-library-validation"), "{main}");
    run_ok(Command::new("/usr/bin/codesign").args(["--verify", "--strict", "--deep"]).arg(&app));

    // The launcher loads the library and runs it in the app role.
    let version = run_ok(Command::new(&launcher).arg("-V"));
    assert_eq!(version.trim(), format!("terminal-decline {}", env!("CARGO_PKG_VERSION")));
    // Through a link, as the cask installs it.
    let link = out.0.join("decline");
    std::os::unix::fs::symlink(&launcher, &link).unwrap();
    assert_eq!(run_ok(Command::new(&link).arg("-V")), version);
    // A subcommand, with a config of its own.
    let listed = run_ok(
        Command::new(&launcher)
            .args(["pin", "--list"])
            .env("XDG_CONFIG_HOME", out.0.join("config"))
            .env("HOME", &out.0),
    );
    assert!(listed.trim().is_empty(), "{listed}");
}

/// `terminal-decline pin` run from the bundled app makes a stub pin from
/// its `Helpers/terminal-decline-pin` (pins.md §3.1), which `--list`
/// shows as a dev pin; `--update` leaves it alone. All in a temp `HOME`.
#[test]
fn the_app_makes_stub_pins() {
    let out = OutDir::new("stub-pins");
    let app = support::bundle(&out.0.join("bundle"), &[]);
    let launcher = support::launcher(&app);
    let apps = out.0.join("Applications");
    let pin = |args: &[&str]| {
        run_ok(
            Command::new(&launcher)
                .arg("pin")
                .args(args)
                .env("XDG_CONFIG_HOME", out.0.join("config"))
                .env("HOME", &out.0),
        )
    };
    let made = pin(&["Rush", "--dir", apps.to_str().unwrap(), "--", "/bin/echo", "hi"]);
    assert!(made.contains("created"), "{made}");
    let rush = apps.join("Rush.app");
    let lsregister = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    let _ = Command::new(lsregister).arg("-u").arg(&rush).status();
    assert_eq!(extract(&rush.join("Contents/Info.plist"), "TDPinId"), "rush");
    assert_eq!(
        extract(&rush.join("Contents/Info.plist"), "TDLibraryRequirement"),
        r#"identifier "red.forbes.terminal-decline.lib""#
    );
    let d = codesign_display(&rush, &["--entitlements", "-"]);
    assert!(d.contains("Identifier=red.forbes.terminal-decline.pin.rush"), "{d}");
    assert!(d.contains("(adhoc,runtime)") && d.contains("disable-library-validation"), "{d}");
    run_ok(Command::new("/usr/bin/codesign").args(["--verify", "--strict"]).arg(&rush));
    // The pin's library requirement passes for this app's library.
    run_ok(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "-R", r#"=identifier "red.forbes.terminal-decline.lib""#])
            .arg(app.join("Contents/Frameworks/libterminal_decline.dylib")),
    );
    let before = codesign_display(&rush, &[]);

    let listed: serde_json::Value = serde_json::from_str(&pin(&["--list", "--json"])).unwrap();
    assert_eq!(listed["pins"][0]["status"], "ok", "{listed}");
    assert_eq!(listed["pins"][0]["dev"], true, "{listed}");
    let updated = pin(&["--update"]);
    assert!(updated.contains("Rush is up to date"), "{updated}");
    assert_eq!(codesign_display(&rush, &[]), before);
}

/// `HARDENED=1` (ad-hoc): the hardened runtime everywhere, and the dev
/// entitlements, without which dyld refuses the ad-hoc library.
#[test]
fn hardened_ad_hoc_bundle_starts() {
    let out = OutDir::new("hardened");
    let app = support::bundle(&out.0, &[("HARDENED", "1"), ("DOGFOOD", "1")]);
    assert!(app.join("Contents/Resources/dogfood").is_file());
    for part in [
        app.clone(),
        app.join("Contents/Frameworks/libterminal_decline.dylib"),
        app.join("Contents/Helpers/terminal-decline-pin"),
    ] {
        let d = codesign_display(&part, &[]);
        assert!(d.contains("(adhoc,runtime)"), "{}: {d}", part.display());
    }
    let main = codesign_display(&app, &["--entitlements", "-"]);
    assert!(main.contains("com.apple.security.cs.disable-library-validation"), "{main}");
    run_ok(Command::new("/usr/bin/codesign").args(["--verify", "--strict", "--deep"]).arg(&app));
    let version = run_ok(Command::new(support::launcher(&app)).arg("-V"));
    assert!(version.starts_with("terminal-decline "), "{version}");
}

/// docs/design/updater.md §5: with Sparkle fetched (scripts/sparkle.sh;
/// macOS CI does), the framework is embedded without its XPC services
/// (folder and symlink), headers and modules, keeps Updater.app, is
/// thinned to the library's architecture and signed; Info.plist has the
/// updater's keys and the committed public key.
#[test]
fn sparkle_is_embedded_thinned_and_signed() {
    let fetched = std::fs::read_dir(support::repo().join("target/sparkle"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.path().join("Sparkle.framework").is_dir());
    if !fetched {
        eprintln!("skipped: no target/sparkle/*/Sparkle.framework (run scripts/sparkle.sh)");
        return;
    }
    let out = OutDir::new("sparkle");
    let app = support::bundle(&out.0, &[("HARDENED", "1")]);
    let fw = app.join("Contents/Frameworks/Sparkle.framework");
    assert!(fw.join("Versions/B/Updater.app/Contents/MacOS/Updater").is_file());
    assert!(fw.join("Versions/B/Autoupdate").is_file());
    for gone in ["XPCServices", "Versions/B/XPCServices", "Headers", "PrivateHeaders", "Modules"] {
        assert!(std::fs::symlink_metadata(fw.join(gone)).is_err(), "{gone} is still there");
    }
    let lib_arch = run_ok(Command::new("/usr/bin/lipo").arg("-archs").arg(&support::parts().lib));
    for bin in [
        "Versions/B/Sparkle",
        "Versions/B/Autoupdate",
        "Versions/B/Updater.app/Contents/MacOS/Updater",
    ] {
        let arch = run_ok(Command::new("/usr/bin/lipo").arg("-archs").arg(fw.join(bin)));
        assert_eq!(arch.trim(), lib_arch.trim(), "{bin}");
    }
    for part in [fw.join("Versions/B/Autoupdate"), fw.join("Versions/B/Updater.app"), fw.clone()] {
        let d = codesign_display(&part, &[]);
        assert!(d.contains("(adhoc,runtime)"), "{}: {d}", part.display());
    }
    run_ok(Command::new("/usr/bin/codesign").args(["--verify", "--strict", "--deep"]).arg(&app));
    let notices = std::fs::read_to_string(app.join("Contents/Resources/THIRD-PARTY-NOTICES.md")).unwrap();
    assert!(notices.contains("## Sparkle") && notices.contains("Andy Matuschak"), "{notices}");
    let plist = app.join("Contents/Info.plist");
    let key = std::fs::read_to_string(support::repo().join("packaging/sparkle-public-key")).unwrap();
    assert_eq!(extract(&plist, "SUPublicEDKey"), key.trim());
    assert_eq!(
        extract(&plist, "SUFeedURL"),
        "https://github.com/0xdeafcafe/terminal-decline/releases/latest/download/appcast.xml"
    );
    for (k, v) in [
        ("SUEnableAutomaticChecks", "true"),
        ("SUScheduledCheckInterval", "86400"),
        ("SUAutomaticallyUpdate", "true"),
        ("SUEnableSystemProfiling", "false"),
        ("SUVerifyUpdateBeforeExtraction", "true"),
        ("SURequireSignedFeed", "true"),
        ("SUScheduledImpatientCheckInterval", "604800"),
    ] {
        assert_eq!(extract(&plist, k), v, "{k}");
    }
    let version = run_ok(Command::new(support::launcher(&app)).arg("-V"));
    assert!(version.starts_with("terminal-decline "), "{version}");
}
