//! Compile the Metal shaders ahead of time so startup doesn't pay for (and
//! keep the memory of) the runtime shader compiler. Falls back to compiling
//! the source at runtime when the Metal toolchain isn't installed, except on
//! CI (the `CI` env var is set), where that is a build error.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=src/shaders.metal");
    println!("cargo::rerun-if-env-changed=CI");
    println!("cargo::rustc-check-cfg=cfg(precompiled_shaders)");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let air = out.join("shaders.air");
    let lib = out.join("shaders.metallib");
    let ok = Command::new("xcrun")
        .args([
            "-sdk",
            "macosx",
            "metal",
            "-O3",
            "-mmacosx-version-min=14.0",
            "-c",
            "src/shaders.metal",
            "-o",
        ])
        .arg(&air)
        .status()
        .is_ok_and(|s| s.success())
        && Command::new("xcrun")
            .args(["-sdk", "macosx", "metallib"])
            .arg(&air)
            .arg("-o")
            .arg(&lib)
            .status()
            .is_ok_and(|s| s.success());
    if ok {
        println!("cargo::rustc-cfg=precompiled_shaders");
    } else if std::env::var_os("CI").is_some() {
        // CI must catch shader errors and a missing toolchain at build time.
        panic!("could not precompile the Metal shaders (CI is set); is the Metal toolchain installed?");
    } else {
        println!("cargo::warning=Metal toolchain not found; shaders will compile at startup");
    }
}
