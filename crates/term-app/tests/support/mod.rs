//! Building Terminal Decline.app for tests: the library (crates/term-dylib),
//! the launcher and the pin stub (crates/term-pin), put together by
//! `scripts/bundle.sh`, all in temp dirs.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// `target/<profile>`, where this test binary's profile builds.
fn profile_dir() -> PathBuf {
    // target/<profile>/deps/<this test>
    std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_path_buf()
}

/// The target dir the parts are built in: `target/bundle-test`, apart
/// from the one the tests run from. A build there, with its own set of
/// packages and features, would rebuild binaries other tests (the smoke
/// tests) are running at the same time. CI keeps it in its cache too.
pub fn bundle_target_dir() -> PathBuf {
    profile_dir().parent().unwrap().join("bundle-test")
}

/// The three parts, built (once per test process) in this test's profile:
/// cargo doesn't build other packages' cdylibs and binaries for a test.
pub struct Parts {
    pub lib: PathBuf,
    pub launcher: PathBuf,
    pub stub: PathBuf,
}

pub fn parts() -> &'static Parts {
    static PARTS: std::sync::OnceLock<Parts> = std::sync::OnceLock::new();
    PARTS.get_or_init(|| {
        let profile = profile_dir();
        let release = profile.file_name().is_some_and(|n| n == "release");
        let target = bundle_target_dir();
        let dir = target.join(if release { "release" } else { "debug" });
        let mut cmd = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        cmd.args(["build", "-p", "term-dylib", "-p", "term-pin", "--lib", "--bins"]);
        if release {
            cmd.arg("--release");
        }
        let status = cmd.env("CARGO_TARGET_DIR", &target).current_dir(repo()).status().unwrap();
        assert!(status.success(), "building the library, launcher and stub");
        let parts = Parts {
            lib: dir.join("libterminal_decline.dylib"),
            launcher: dir.join("terminal-decline-launcher"),
            stub: dir.join("terminal-decline-pin"),
        };
        for p in [&parts.lib, &parts.launcher, &parts.stub] {
            assert!(p.is_file(), "{} not built", p.display());
        }
        parts
    })
}

/// `scripts/bundle.sh` into `out_dir` with `env` (e.g. `HARDENED=1`),
/// ad-hoc signed; the .app it made.
pub fn bundle(out_dir: &Path, env: &[(&str, &str)]) -> PathBuf {
    let p = parts();
    std::fs::create_dir_all(out_dir).unwrap();
    let mut cmd = Command::new("/bin/bash");
    cmd.arg(repo().join("scripts/bundle.sh"))
        .arg("--lib")
        .arg(&p.lib)
        .arg("--launcher")
        .arg(&p.launcher)
        .arg("--stub")
        .arg(&p.stub)
        .arg(out_dir)
        .args(["1.2.3", "7"])
        .env_remove("DOGFOOD")
        .env_remove("HARDENED")
        .env_remove("CODESIGN_IDENTITY");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let made = cmd.output().unwrap();
    assert!(made.status.success(), "bundle.sh: {}", String::from_utf8_lossy(&made.stderr));
    PathBuf::from(String::from_utf8(made.stdout).unwrap().trim())
}

/// The bundle's launcher, `Contents/MacOS/terminal-decline`.
pub fn launcher(app: &Path) -> PathBuf {
    app.join("Contents/MacOS/terminal-decline")
}
