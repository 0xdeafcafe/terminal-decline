//! The app bundle `scripts/bundle.sh` makes: its Info.plist is valid and
//! declares what the app answers (folders opened with it, the Services
//! menu items, named with [`terminal_decline::PRODUCT_NAME`]).
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::Command;

const APP: &str = env!("CARGO_BIN_EXE_terminal-decline");

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

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

#[test]
fn info_plist_declares_folders_and_services() {
    let out_dir = std::env::temp_dir().join(format!("terminal-decline-bundle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir).unwrap();
    let made = Command::new("/bin/bash")
        .arg(repo().join("scripts/bundle.sh"))
        .arg(APP)
        .arg(&out_dir)
        .args(["1.2.3", "7"])
        .env_remove("DOGFOOD")
        .env_remove("CODESIGN_IDENTITY")
        .output()
        .unwrap();
    assert!(made.status.success(), "bundle.sh: {}", String::from_utf8_lossy(&made.stderr));
    let app = PathBuf::from(String::from_utf8(made.stdout).unwrap().trim());
    let plist = app.join("Contents/Info.plist");
    let lint = Command::new("/usr/bin/plutil").arg("-lint").arg(&plist).output().unwrap();
    assert!(lint.status.success(), "{}", String::from_utf8_lossy(&lint.stdout));

    assert_eq!(extract(&plist, "CFBundleDocumentTypes.0.LSItemContentTypes.0"), "public.folder");
    let bundle_name = extract(&plist, "CFBundleName");
    for (i, (message, title)) in [("newWindowAtFolder", "Window"), ("newTabAtFolder", "Tab")].into_iter().enumerate() {
        let service = |key: &str| extract(&plist, &format!("NSServices.{i}.{key}"));
        // The app's delegate answers `<message>:userData:error:`.
        assert_eq!(service("NSMessage"), message);
        assert_eq!(
            service("NSMenuItem.default"),
            format!("New {} {title} at Folder", terminal_decline::PRODUCT_NAME)
        );
        assert_eq!(service("NSPortName"), bundle_name);
        assert_eq!(service("NSSendFileTypes.0"), "public.folder");
    }
    std::fs::remove_dir_all(&out_dir).unwrap();
}
