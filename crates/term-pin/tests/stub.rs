//! The pin stub against the C fixture library (docs/design/pins.md §9):
//! fake apps and pins in a temp dir, `HOME` pointing there, a random app
//! bundle id per test so LaunchServices can never find a real app.
#![cfg(target_os = "macos")]

mod common;

use std::process::Command;

use common::*;

const APP_NAME: &str = "Terminal Decline.app";

/// A pin `id` whose app is `~/Applications/Terminal Decline.app` holding
/// a fixture of `build`.
fn setup(s: &Scratch, id: &str, build: &str) -> (String, std::path::PathBuf) {
    let app_id = test_app_id();
    let lib = fixture(&s.path(&format!("lib/{build}.dylib")), build);
    app(&s.home().join("Applications").join(APP_NAME), &app_id, &lib);
    let p = s.path(&format!("pins/{id}.app"));
    pin(&p, id, &app_id, &dev_requirement());
    (app_id, p)
}

#[test]
fn runs_the_library_with_every_field() {
    let s = Scratch::new("fields");
    let (_, p) = setup(&s, "rush", "A");
    let out = s.path("out");
    let o = run_pin(&p, "rush", &s.home(), &out);
    assert_eq!(o.status.code(), Some(7), "{}", stderr(&o));
    let r = report(&out);
    assert_eq!(one(&r, "entry"), "td_pin_main_v1");
    assert_eq!(one(&r, "build"), "A");
    assert_eq!(one(&r, "size"), size_of::<term_pin::abi::PinInfoV1>().to_string());
    assert_eq!(one(&r, "abi"), "1");
    assert_eq!(one(&r, "pin_id"), "rush");
    assert_eq!(one(&r, "pin_bundle"), p.to_str().unwrap());
    assert_eq!(
        one(&r, "app_bundle"),
        s.home().join("Applications").join(APP_NAME).to_str().unwrap()
    );
    assert_eq!(one(&r, "stub_version"), "1");
    assert_eq!(one(&r, "argc"), "2");
    assert_eq!(r["argv"], [p.join("Contents/MacOS/rush").to_str().unwrap(), out.to_str().unwrap()]);
    assert_eq!(stderr(&o), "");
}

#[test]
fn the_override_file_wins_and_must_be_absolute() {
    let s = Scratch::new("override");
    let (app_id, p) = setup(&s, "ovr", "home");
    let other = s.path("elsewhere/Terminal Decline.app");
    app(&other, &app_id, &fixture(&s.path("lib/over.dylib"), "over"));
    let cfg = s.home().join(".config/terminal-decline");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(cfg.join("pin-app"), format!("{}\n", other.display())).unwrap();
    let out = s.path("out");
    let o = run_pin(&p, "ovr", &s.home(), &out);
    assert_eq!(o.status.code(), Some(7), "{}", stderr(&o));
    assert_eq!(one(&report(&out), "build"), "over");
    assert_eq!(one(&report(&out), "app_bundle"), other.to_str().unwrap());

    // Relative: refused, so ~/Applications is used, and stderr is quiet
    // (only a failed start says anything).
    std::fs::write(cfg.join("pin-app"), "elsewhere/Terminal Decline.app\n").unwrap();
    let o = run_pin(&p, "ovr", &s.home(), &out);
    assert_eq!(o.status.code(), Some(7), "{}", stderr(&o));
    assert_eq!(one(&report(&out), "build"), "home");

    // Relative and nothing else: the reason is named.
    std::fs::remove_dir_all(s.home().join("Applications")).unwrap();
    let o = run_pin(&p, "ovr", &s.home(), &out);
    assert_eq!(o.status.code(), Some(1));
    let e = stderr(&o);
    assert!(e.contains("Terminal Decline isn't installed"), "{e}");
    assert!(
        e.contains("pin-app names \"elsewhere/Terminal Decline.app\", which isn't an absolute path"),
        "{e}"
    );
}

#[test]
fn an_app_with_another_bundle_id_is_skipped() {
    let s = Scratch::new("bundleid");
    let (app_id, p) = setup(&s, "bid", "home");
    let other = s.path("other/Terminal Decline.app");
    app(&other, "red.forbes.someone-else", &fixture(&s.path("lib/x.dylib"), "x"));
    let cfg = s.home().join(".config/terminal-decline");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(cfg.join("pin-app"), format!("{}\n", other.display())).unwrap();
    let out = s.path("out");
    let o = run_pin(&p, "bid", &s.home(), &out);
    assert_eq!(o.status.code(), Some(7), "{}", stderr(&o));
    assert_eq!(one(&report(&out), "build"), "home");

    std::fs::remove_dir_all(s.home().join("Applications")).unwrap();
    let o = run_pin(&p, "bid", &s.home(), &out);
    assert_eq!(o.status.code(), Some(1));
    let e = stderr(&o);
    assert!(e.contains("isn't installed"), "{e}");
    assert!(
        e.contains(&format!("{}: is red.forbes.someone-else, not {app_id}", other.display())),
        "{e}"
    );
}

#[test]
fn a_library_with_another_identifier_is_refused() {
    let s = Scratch::new("ident");
    let (app_id, p) = setup(&s, "idt", "A");
    let lib = fixture_signed(&s.path("lib/other.dylib"), &[], "com.example.other", "-");
    let a = s.home().join("Applications").join(APP_NAME);
    app(&a, &app_id, &lib);
    let out = s.path("out");
    let o = run_pin(&p, "idt", &s.home(), &out);
    assert_eq!(o.status.code(), Some(1));
    assert!(!out.exists());
    let e = stderr(&o);
    let lib_path = a.join("Contents/Frameworks/libterminal_decline.dylib");
    assert!(
        e.contains(&format!(
            "The Terminal Decline library at {} fails its signature check",
            lib_path.display()
        )),
        "{e}"
    );
    assert!(
        e.contains("isn't signed as the pin requires (identifier \"red.forbes.terminal-decline.lib\")"),
        "{e}"
    );
}

#[test]
fn a_library_changed_after_signing_is_refused() {
    let s = Scratch::new("tamper");
    let (_, p) = setup(&s, "tmp", "TAMPERME");
    let lib = s
        .home()
        .join("Applications")
        .join(APP_NAME)
        .join("Contents/Frameworks/libterminal_decline.dylib");
    let mut bytes = std::fs::read(&lib).unwrap();
    let at = bytes.windows(8).position(|w| w == b"TAMPERME").expect("the build string");
    bytes[at] = b'X';
    std::fs::write(&lib, bytes).unwrap();
    let out = s.path("out");
    let o = run_pin(&p, "tmp", &s.home(), &out);
    assert_eq!(o.status.code(), Some(1));
    assert!(!out.exists());
    let e = stderr(&o);
    assert!(e.contains("fails its signature check"), "{e}");
    assert!(e.contains(lib.to_str().unwrap()), "{e}");
}

#[test]
fn a_library_without_this_pins_interface_needs_a_newer_app() {
    let s = Scratch::new("newer");
    let (app_id, p) = setup(&s, "nwr", "A");
    let lib = fixture_signed(&s.path("lib/v2.dylib"), &["-DNO_V1"], LIBRARY_ID, "-");
    app(&s.home().join("Applications").join(APP_NAME), &app_id, &lib);
    let out = s.path("out");
    let o = run_pin(&p, "nwr", &s.home(), &out);
    assert_eq!(o.status.code(), Some(1), "td_pin_main_v2 must not be called");
    let e = stderr(&o);
    assert!(e.contains("Test Pin needs a newer Terminal Decline"), "{e}");
    assert!(e.contains("brew upgrade --greedy terminal-decline"), "{e}");
}

#[test]
fn no_app_anywhere() {
    let s = Scratch::new("none");
    let p = s.path("pins/none.app");
    pin(&p, "none", &test_app_id(), &dev_requirement());
    let o = run_pin(&p, "none", &s.home(), &s.path("out"));
    assert_eq!(o.status.code(), Some(1));
    let e = stderr(&o);
    assert!(
        e.starts_with("Test Pin: Test Pin is a Terminal Decline pin, and Terminal Decline isn't installed."),
        "{e}"
    );
}

#[test]
fn a_pin_missing_a_key_is_damaged() {
    let s = Scratch::new("damaged");
    let p = s.path("pins/dmg.app");
    let mut keys = pin_keys("dmg", &test_app_id(), &dev_requirement());
    keys.retain(|(k, _)| *k != "TDLibraryRequirement");
    pin_with(&p, "dmg", &keys);
    let o = run_pin(&p, "dmg", &s.home(), &s.path("out"));
    assert_eq!(o.status.code(), Some(1));
    let e = stderr(&o);
    assert!(e.contains("Test Pin is damaged (no TDLibraryRequirement in its Info.plist)"), "{e}");
    assert!(e.contains("terminal-decline pin --repair"), "{e}");

    let mut keys = pin_keys("dmg", &test_app_id(), &dev_requirement());
    for (k, v) in &mut keys {
        if *k == "TDPinAbi" {
            *v = "1".into(); // a string, not an integer
        }
    }
    pin_with(&p, "dmg", &keys);
    let e = stderr(&run_pin(&p, "dmg", &s.home(), &s.path("out")));
    assert!(e.contains("is damaged (no TDPinAbi"), "{e}");
}

#[test]
fn dyld_insert_libraries_is_ignored() {
    // The hardened runtime's refusal is enforced by System Integrity
    // Protection; CI's virtual machines run with it off.
    let sip = std::process::Command::new("/usr/bin/csrutil").arg("status").output();
    if !sip.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("enabled")) {
        eprintln!("skipped: System Integrity Protection is off, so DYLD_INSERT_LIBRARIES isn't blocked");
        return;
    }
    let s = Scratch::new("dyld");
    let (_, p) = setup(&s, "dyl", "A");
    let inject = s.path("lib/inject.dylib");
    fixture_signed(&inject, &["-DINJECT"], "inject", "-");
    let marker = s.path("injected");
    let out = s.path("out");
    let o = run(
        &p.join("Contents/MacOS/dyl"),
        &s.home(),
        &[&out],
        &[("DYLD_INSERT_LIBRARIES", &inject), ("TD_FIXTURE_INJECTED", &marker)],
    );
    assert_eq!(o.status.code(), Some(7), "{}", stderr(&o));
    assert!(!marker.exists(), "the hardened stub loaded an inserted library");

    // The check is real: a plain (not hardened) copy of the stub honours it.
    let plain = s.path("plain");
    std::fs::copy(stub(), &plain).unwrap();
    assert!(
        Command::new("codesign")
            .args(["-f", "-s", "-"])
            .arg(&plain)
            .status()
            .unwrap()
            .success()
    );
    let o = run(
        &plain,
        &s.home(),
        &[],
        &[("DYLD_INSERT_LIBRARIES", &inject), ("TD_FIXTURE_INJECTED", &marker)],
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(marker.exists(), "a plain stub should honour DYLD_INSERT_LIBRARIES");
}

#[test]
fn the_struct_matches_the_header() {
    let s = Scratch::new("layout");
    let exe = s.path("layout");
    let o = Command::new("xcrun")
        .args(["clang", "-DLAYOUT", "-o"])
        .arg(&exe)
        .arg(FIXTURE)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
    let text = String::from_utf8(Command::new(&exe).output().unwrap().stdout).unwrap();
    use std::mem::offset_of;
    use term_pin::abi::PinInfoV1 as P;
    let want = format!(
        "size={}\nsize_off={}\nabi_off={}\npin_id_off={}\npin_bundle_off={}\napp_bundle_off={}\nstub_version_off={}\nargc_off={}\nargv_off={}\n",
        size_of::<P>(),
        offset_of!(P, size),
        offset_of!(P, abi),
        offset_of!(P, pin_id),
        offset_of!(P, pin_bundle),
        offset_of!(P, app_bundle),
        offset_of!(P, stub_version),
        offset_of!(P, argc),
        offset_of!(P, argv),
    );
    assert_eq!(text, want);
}

/// P.2's "done when": a pin's identity stays the same across an app
/// upgrade, and the new library is what runs.
#[test]
fn identity_stays_across_upgrades() {
    let s = Scratch::new("identity");
    let (app_id, p) = setup(&s, "ident", "A");
    let out = s.path("out");
    assert_eq!(run_pin(&p, "ident", &s.home(), &out).status.code(), Some(7));
    assert_eq!(one(&report(&out), "build"), "A");
    let before = cdhash(&p);

    let a = s.home().join("Applications").join(APP_NAME);
    std::fs::remove_dir_all(&a).unwrap();
    app(&a, &app_id, &fixture(&s.path("lib/B.dylib"), "B"));
    let o = run_pin(&p, "ident", &s.home(), &out);
    assert_eq!(o.status.code(), Some(7), "{}", stderr(&o));
    assert_eq!(one(&report(&out), "build"), "B");
    assert_eq!(cdhash(&p), before);
    let v = Command::new("codesign").args(["--verify", "--strict"]).arg(&p).output().unwrap();
    assert!(v.status.success(), "{}", stderr(&v));
}

/// The release requirement with a real Developer ID certificate. Runs
/// only when `TD_PIN_TEST_IDENTITY` names one in the keychain (CI has
/// none until N.4); its team is read from the name's "(TEAMID)".
#[test]
fn team_requirement_with_a_real_identity() {
    let Some(identity) = std::env::var("TD_PIN_TEST_IDENTITY").ok().filter(|i| !i.is_empty()) else {
        eprintln!("skipped: TD_PIN_TEST_IDENTITY not set");
        return;
    };
    let team = identity
        .rsplit_once('(')
        .and_then(|(_, t)| t.strip_suffix(')'))
        .expect("identity ends in (TEAMID)");
    let s = Scratch::new("team");
    let app_id = test_app_id();
    let a = s.home().join("Applications").join(APP_NAME);
    let signed = fixture_signed(&s.path("lib/rel.dylib"), &["-DBUILD=\"rel\""], LIBRARY_ID, &identity);
    app(&a, &app_id, &signed);
    let p = s.path("pins/rel.app");
    pin(&p, "rel", &app_id, &term_pin::abi::team_requirement(team));
    let out = s.path("out");
    let o = run_pin(&p, "rel", &s.home(), &out);
    assert_eq!(o.status.code(), Some(7), "{}", stderr(&o));
    assert_eq!(one(&report(&out), "build"), "rel");

    // Ad-hoc with the right identifier: refused by a release pin.
    std::fs::remove_dir_all(&a).unwrap();
    app(&a, &app_id, &fixture(&s.path("lib/adhoc.dylib"), "adhoc"));
    let o = run_pin(&p, "rel", &s.home(), &out);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("fails its signature check"), "{}", stderr(&o));

    // Another team: refused.
    std::fs::remove_dir_all(&a).unwrap();
    app(&a, &app_id, &signed);
    pin(&p, "rel", &app_id, &term_pin::abi::team_requirement("ABCDE12345"));
    assert_eq!(run_pin(&p, "rel", &s.home(), &out).status.code(), Some(1));
}
