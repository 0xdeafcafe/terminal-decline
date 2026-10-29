//! The app's launcher (docs/design/pins.md §2.2, §9).
#![cfg(target_os = "macos")]

mod common;

use common::*;

/// A fake app with the launcher as its executable and, if `lib`, the
/// fixture as its library.
fn bundle(s: &Scratch, lib: bool) -> std::path::PathBuf {
    let a = s.path("Terminal Decline.app");
    std::fs::create_dir_all(a.join("Contents/MacOS")).unwrap();
    std::fs::create_dir_all(a.join("Contents/Frameworks")).unwrap();
    let exe = a.join("Contents/MacOS/terminal-decline");
    std::fs::copy(LAUNCHER, &exe).unwrap();
    if lib {
        let f = fixture(&s.path("lib/app.dylib"), "app");
        std::fs::copy(f, a.join("Contents/Frameworks/libterminal_decline.dylib")).unwrap();
    }
    exe
}

#[test]
fn finds_the_library_through_a_symlink() {
    let s = Scratch::new("launcher-link");
    let exe = bundle(&s, true);
    // As the cask links it: /opt/homebrew/bin/terminal-decline -> the app's.
    let link = s.path("bin/decline");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&exe, &link).unwrap();
    let out = s.path("out");
    let o = run(&link, &s.home(), &[&out], &[]);
    assert_eq!(o.status.code(), Some(9), "{}", stderr(&o));
    let r = report(&out);
    assert_eq!(one(&r, "entry"), "td_app_main_v1");
    assert_eq!(one(&r, "build"), "app");
    assert_eq!(one(&r, "argc"), "2");
    assert_eq!(r["argv"][1], out.to_str().unwrap());
}

#[test]
fn a_missing_library_is_reported() {
    let s = Scratch::new("launcher-missing");
    let exe = bundle(&s, false);
    let o = run(&exe, &s.home(), &[], &[]);
    assert_eq!(o.status.code(), Some(1));
    let lib = s.path("Terminal Decline.app/Contents/Frameworks/libterminal_decline.dylib");
    assert_eq!(
        stderr(&o),
        format!(
            "terminal-decline: The Terminal Decline library at {} is missing. Reinstall Terminal Decline.\n",
            lib.display()
        )
    );
}

#[test]
fn a_library_without_the_entry_point_is_reported() {
    let s = Scratch::new("launcher-entry");
    let exe = bundle(&s, false);
    let lib = s.path("Terminal Decline.app/Contents/Frameworks/libterminal_decline.dylib");
    fixture_signed(&lib, &["-DINJECT"], LIBRARY_ID, "-");
    let o = run(&exe, &s.home(), &[], &[]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("has no td_app_main_v1"), "{}", stderr(&o));
}
