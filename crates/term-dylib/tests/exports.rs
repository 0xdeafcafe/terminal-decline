//! The library exports exactly the `td_*` entry points of the ABI header,
//! under the install name the launcher and pins expect (pins.md §2.5).
#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::process::Command;

/// `libterminal_decline.dylib`, built by cargo for this profile: cargo
/// doesn't build a cdylib for its own crate's integration tests.
fn dylib() -> PathBuf {
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT
        .get_or_init(|| {
            // target/<profile>/deps/<this test> → target/<profile>
            let profile_dir = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_path_buf();
            let mut cmd = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
            cmd.args(["build", "-p", "term-dylib", "--lib"]);
            if profile_dir.file_name().is_some_and(|n| n == "release") {
                cmd.arg("--release");
            }
            let status = cmd.current_dir(env!("CARGO_MANIFEST_DIR")).status().unwrap();
            assert!(status.success(), "building term-dylib");
            let lib = profile_dir.join("libterminal_decline.dylib");
            assert!(lib.is_file(), "{} not built", lib.display());
            lib
        })
        .clone()
}

fn run(cmd: &str, args: &[&str]) -> String {
    let out = Command::new(cmd).args(args).arg(dylib()).output().unwrap();
    assert!(out.status.success(), "{cmd}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

/// The entry points, and otherwise only the statics objc2's
/// `define_class!` gives each of our classes an `export_name` (rustc
/// exports those from a cdylib, and ld64 can't take them out of rustc's
/// export list: `-unexported_symbol` refuses to combine with it).
#[test]
fn exports_only_the_entry_points() {
    let names: Vec<String> = run("/usr/bin/nm", &["-gU", "-j"])
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    let mut entries: Vec<&str> = names.iter().map(String::as_str).filter(|n| n.starts_with("_td_")).collect();
    entries.sort();
    assert_eq!(entries, ["_td_app_main_v1", "_td_pin_main_v1"]);
    let objc2 = ["___CLASS_", "___DROP_FLAG_OFFSET_", "___IVAR_OFFSET_", "___REGISTER_CLASS_"];
    for n in names.iter().filter(|n| !n.starts_with("_td_")) {
        let class = objc2.iter().find_map(|p| n.strip_prefix(p));
        assert!(class.is_some_and(|c| c.starts_with("Decline")), "unexpected export {n}");
    }
}

#[test]
fn install_name_is_rpath_relative() {
    // `otool -D` prints the file, then its install name.
    let out = run("/usr/bin/otool", &["-D"]);
    assert_eq!(out.lines().nth(1).map(str::trim), Some("@rpath/libterminal_decline.dylib"), "{out}");
}
