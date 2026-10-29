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
