//! Helpers for the stub and launcher tests: fake apps and pins in a temp
//! dir, the C fixture library, and running the built binaries with a test
//! `HOME`. Every pin carries its own random app bundle id, so
//! LaunchServices (system-wide) never finds a real installed app.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The stub under test: the one this build made, or `TD_PIN_TEST_STUB`
/// (the release-stub build, or a stub from an earlier release run against
/// today's code: pins.md §9, "Released stubs").
pub fn stub() -> PathBuf {
    std::env::var_os("TD_PIN_TEST_STUB")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_terminal-decline-pin")))
}
pub const LAUNCHER: &str = env!("CARGO_BIN_EXE_terminal-decline-launcher");
pub const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixture/fixture.c");
pub const LIBRARY_ID: &str = term_pin::abi::LIBRARY_ID;

/// A temp dir, removed when dropped.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("term-pin-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        Scratch(dir.canonicalize().unwrap())
    }

    pub fn path(&self, p: &str) -> PathBuf {
        self.0.join(p)
    }

    pub fn home(&self) -> PathBuf {
        self.0.join("home")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn check(cmd: &mut Command) -> Output {
    let out = cmd.output().unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
    assert!(out.status.success(), "{cmd:?}: {}", String::from_utf8_lossy(&out.stderr));
    out
}

/// A random bundle id for the fake app, so nothing real can match.
pub fn test_app_id() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("red.forbes.terminal-decline.test.{}{t}", std::process::id())
}

/// Compiles the fixture with `defines` into `out` and signs it ad-hoc
/// with `identifier` (`identity` "-"), or with a real identity.
pub fn fixture_signed(out: &Path, defines: &[&str], identifier: &str, identity: &str) -> PathBuf {
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    check(
        Command::new("xcrun")
            .args(["clang", "-dynamiclib", "-O1"])
            .args(defines)
            .arg("-o")
            .arg(out)
            .arg(FIXTURE),
    );
    let mut sign = Command::new("codesign");
    sign.args(["-f", "-s", identity, "-i", identifier]);
    if identity != "-" {
        sign.args(["-o", "runtime", "--timestamp"]);
    }
    check(sign.arg(out));
    out.to_path_buf()
}

/// The fixture reporting `build`, ad-hoc signed as the library.
pub fn fixture(out: &Path, build: &str) -> PathBuf {
    fixture_signed(out, &[&format!("-DBUILD=\"{build}\"")], LIBRARY_ID, "-")
}

/// A fake Terminal Decline.app at `app` with `bundle_id`, holding `lib`.
pub fn app(app: &Path, bundle_id: &str, lib: &Path) {
    std::fs::create_dir_all(app.join("Contents/Frameworks")).unwrap();
    std::fs::write(
        app.join("Contents/Info.plist"),
        plist(&[("CFBundleIdentifier", bundle_id.into()), ("CFBundleVersion", "1".into())]),
    )
    .unwrap();
    std::fs::copy(lib, app.join("Contents/Frameworks/libterminal_decline.dylib")).unwrap();
}

pub enum V {
    S(String),
    I(i64),
}

impl From<&str> for V {
    fn from(s: &str) -> V {
        V::S(s.into())
    }
}

pub fn plist(entries: &[(&str, V)]) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n",
    );
    for (k, v) in entries {
        let v = match v {
            V::S(t) => format!(
                "<string>{}</string>",
                t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
            ),
            V::I(i) => format!("<integer>{i}</integer>"),
        };
        s += &format!("<key>{k}</key>{v}\n");
    }
    s + "</dict></plist>\n"
}

/// What a pin's Info.plist holds (pins.md §3.1), the TD keys from
/// arguments.
pub fn pin_keys(id: &str, app_id: &str, requirement: &str) -> Vec<(&'static str, V)> {
    vec![
        (
            "CFBundleIdentifier",
            format!("red.forbes.terminal-decline.pin.{id}").as_str().into(),
        ),
        ("CFBundleExecutable", id.into()),
        ("CFBundleName", "Test Pin".into()),
        ("CFBundlePackageType", "APPL".into()),
        ("CFBundleShortVersionString", "1".into()),
        ("CFBundleVersion", "1".into()),
        ("TDPinAbi", V::I(1)),
        ("TDPinId", id.into()),
        ("TDAppBundleId", app_id.into()),
        ("TDLibraryRequirement", requirement.into()),
    ]
}

/// A pin at `pin` around the built stub with `keys`, signed as pins are:
/// ad-hoc, hardened, library validation off.
pub fn pin_with(pin: &Path, id: &str, keys: &[(&str, V)]) {
    let _ = std::fs::remove_dir_all(pin);
    std::fs::create_dir_all(pin.join("Contents/MacOS")).unwrap();
    std::fs::copy(stub(), pin.join("Contents/MacOS").join(id)).unwrap();
    std::fs::write(pin.join("Contents/Info.plist"), plist(keys)).unwrap();
    let ents = pin.parent().unwrap().join(format!("{id}-ents.plist"));
    std::fs::write(
        &ents,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
         <key>com.apple.security.cs.disable-library-validation</key><true/></dict></plist>\n",
    )
    .unwrap();
    check(
        Command::new("codesign")
            .args(["-f", "-s", "-", "-o", "runtime", "--entitlements"])
            .arg(&ents)
            .args(["-i", &format!("red.forbes.terminal-decline.pin.{id}")])
            .arg(pin),
    );
}

pub fn pin(pin: &Path, id: &str, app_id: &str, requirement: &str) {
    pin_with(pin, id, &pin_keys(id, app_id, requirement));
}

pub fn dev_requirement() -> String {
    term_pin::abi::dev_requirement()
}

/// Runs `exe` with `HOME` = `home`, no alerts, and `args`.
pub fn run(exe: &Path, home: &Path, args: &[&Path], env: &[(&str, &Path)]) -> Output {
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .env("HOME", home)
        .env("TD_PIN_NO_ALERT", "1")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("XDG_CONFIG_HOME");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

/// Runs the pin's stub with the fixture's output file as its argument.
pub fn run_pin(pin: &Path, id: &str, home: &Path, out: &Path) -> Output {
    let _ = std::fs::remove_file(out);
    run(&pin.join("Contents/MacOS").join(id), home, &[out], &[])
}

/// The fixture's `key=value` lines.
pub fn report(out: &Path) -> HashMap<String, Vec<String>> {
    let text = std::fs::read_to_string(out).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    let mut m: HashMap<String, Vec<String>> = HashMap::new();
    for l in text.lines() {
        let (k, v) = l.split_once('=').unwrap();
        m.entry(k.into()).or_default().push(v.into());
    }
    m
}

pub fn one(r: &HashMap<String, Vec<String>>, key: &str) -> String {
    r.get(key).unwrap_or_else(|| panic!("no {key} in {r:?}"))[0].clone()
}

pub fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// The code hash macOS identifies an ad-hoc bundle by.
pub fn cdhash(bundle: &Path) -> String {
    let o = Command::new("codesign").arg("-dvvv").arg(bundle).output().unwrap();
    let text = String::from_utf8_lossy(&o.stderr).into_owned();
    text.lines()
        .find_map(|l| l.strip_prefix("CDHash="))
        .unwrap_or_else(|| panic!("no CDHash: {text}"))
        .to_string()
}
