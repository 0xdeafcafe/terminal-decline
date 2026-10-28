//! Smoke tests of the real app: they open windows, so they are `#[ignore]`d
//! and run one at a time (nextest group `gui`). Run them with
//! `cargo nextest run -p decline --run-ignored only -E 'binary(smoke)'`.
//!
//! Each launches the built `decline` with `DECLINE_TEST_DUMP=<file>`, so a
//! program in the window can drive it with the private control OSC (see
//! `decline::control`): `dump` writes a line of JSON after the next frame
//! the GPU has finished, then `resize`, `font` and `quit`. The tests assert
//! on the dumps, not on pixels. Config comes from an empty temp dir
//! (`XDG_CONFIG_HOME`), never the user's.
//!
//! On CI (`CI` set) a missing Metal device fails the test; locally it is
//! skipped. A launch failure always fails.
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

use serde_json::Value;

const APP: &str = env!("CARGO_BIN_EXE_decline");
/// Longest a scenario may take before the app is killed.
const TIMEOUT: Duration = Duration::from_secs(30);

/// `printf` arguments for a control command.
fn osc(cmd: &str) -> String {
    format!("printf '\\033]7770;{cmd}\\007'; ")
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("decline-smoke-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("config")).unwrap();
        TempDir(dir.canonicalize().unwrap())
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn dump_file(&self) -> PathBuf {
        self.0.join("dump.jsonl")
    }

    fn config_home(&self) -> PathBuf {
        self.0.join("config")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn on_ci() -> bool {
    std::env::var_os("CI").is_some_and(|v| !v.is_empty())
}

/// Whether the GPU tests can run here.
fn have_metal() -> bool {
    if objc2_metal::MTLCreateSystemDefaultDevice().is_some() {
        return true;
    }
    assert!(!on_ci(), "no Metal device on CI");
    eprintln!("skipped: no Metal device");
    false
}

/// Run `cmd` to completion, killing it after [`TIMEOUT`].
fn run(cmd: &mut Command) -> (ExitStatus, Duration) {
    let start = Instant::now();
    let mut child = cmd.spawn().unwrap_or_else(|e| panic!("launching {cmd:?}: {e}"));
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return (status, start.elapsed());
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{cmd:?} still running after {TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `decline -e sh -c <script>` with the dump file, a temp config home and
/// any `env`.
fn decline(dir: &TempDir, script: &str, dump: bool) -> (ExitStatus, Duration) {
    let mut cmd = Command::new(APP);
    cmd.args(["-e", "/bin/sh", "-c", script])
        .env("XDG_CONFIG_HOME", dir.config_home())
        .env_remove("DECLINE_TEST_DUMP")
        .current_dir(dir.path());
    if dump {
        cmd.env("DECLINE_TEST_DUMP", dir.dump_file());
    }
    run(&mut cmd)
}

fn dumps(dir: &TempDir) -> Vec<Value> {
    let text = std::fs::read_to_string(dir.dump_file()).unwrap_or_default();
    text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn num(d: &Value, key: &str) -> f64 {
    d[key].as_f64().unwrap_or_else(|| panic!("no {key} in {d}"))
}

fn screen(d: &Value) -> &str {
    d["screen"].as_str().unwrap()
}

/// Checks every dump must pass: the GPU finished frames, glyphs were drawn,
/// and the stats are there.
fn check_dump(d: &Value) {
    assert!(num(d, "frames") >= 1.0, "{d}");
    assert!(num(d, "glyphs") > 0.0, "{d}");
    assert!(num(d, "mask_texture") >= 256.0 && num(d, "color_texture") >= 64.0, "{d}");
    assert!(num(d, "phys_footprint") > 1e6, "{d}");
    assert!(num(d, "scale") >= 1.0, "{d}");
    assert!(num(d, "cell_width") > 0.0 && num(d, "cell_height") > 0.0, "{d}");
}

#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_program_output_is_drawn() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("output");
    let script = format!(
        "echo 'hello from smoke'; printf 'wide: 日本語\\n'; {}{}sleep 20",
        osc("dump"),
        osc("quit")
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 1, "{d:?}");
    check_dump(&d[0]);
    assert_eq!(d[0]["seq"], 1);
    let s = screen(&d[0]);
    assert!(s.starts_with("hello from smoke|\nwide: 日本語|\n"), "{s}");
    // "hello from smoke" and "wide: " plus three wide glyphs, at least.
    assert!(num(&d[0], "glyphs") >= 17.0, "{}", d[0]);
}

#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_resize_through_the_control_osc() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("resize");
    // The resize happens on the app's main thread, so the program waits
    // (at most 5 s) for its terminal to change size before printing it.
    let wait = "i=0; while [ \"$(stty size)\" != '15 60' ] && [ $i -lt 100 ]; do sleep 0.05; i=$((i+1)); done; ";
    let script = format!(
        "echo resizing; {}{}{}{wait}stty size; {}{}sleep 20",
        osc("dump"),
        osc("resize 60 15"),
        osc("dump"),
        osc("dump"),
        osc("quit")
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 3, "{d:?}");
    for x in &d {
        check_dump(x);
    }
    let grid = |x: &Value| (num(x, "cols") as u32, num(x, "lines") as u32);
    assert_ne!(grid(&d[0]), (60, 15));
    assert_eq!(grid(&d[1]), (60, 15));
    assert!(num(&d[1], "frames") > num(&d[0], "frames"));
    // The program was told: stty sees the new size.
    assert!(screen(&d[2]).contains("15 60|"), "{}", screen(&d[2]));
}

#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_font_bigger_changes_the_cells() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("font");
    let script = format!("echo text; {}{}{}{}sleep 20", osc("dump"), osc("font +1"), osc("dump"), osc("quit"));
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 2, "{d:?}");
    check_dump(&d[0]);
    check_dump(&d[1]);
    assert_eq!(num(&d[1], "font_size"), num(&d[0], "font_size") + 1.0);
    assert!(num(&d[1], "cell_height") > num(&d[0], "cell_height"), "{} vs {}", d[0], d[1]);
    assert!(num(&d[1], "cell_width") >= num(&d[0], "cell_width"));
    // The window stays the same size, so the grid shrinks. At 1x scale a
    // point more can round to the same cell width, so only one direction is
    // guaranteed to shrink; neither may grow.
    let (c0, c1) = (num(&d[0], "cols"), num(&d[1], "cols"));
    let (l0, l1) = (num(&d[0], "lines"), num(&d[1], "lines"));
    assert!(c1 <= c0 && l1 <= l0 && (c1 < c0 || l1 < l0), "{} vs {}", d[0], d[1]);
}

/// Without `DECLINE_TEST_DUMP` the control OSC is ignored: `quit` doesn't
/// quit, the app ends when the program does.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_control_osc_ignored_without_the_env_var() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("ignored");
    let script = format!("{}{}echo still here; sleep 2", osc("dump"), osc("quit"));
    let (status, elapsed) = decline(&dir, &script, false);
    assert!(status.success(), "{status}");
    assert!(elapsed >= Duration::from_secs(2), "quit after {elapsed:?}");
    assert!(!dir.dump_file().exists());
}

/// A pin bundle made by the pin module in a temp dir, with a unique bundle
/// id, launched the way the Dock does (LaunchServices, via `open`).
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_pin_bundle_launches() {
    use decline::pin::{self, Options};
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("pin");
    let unique = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );
    let paths = decline::Paths {
        config_dir: dir.config_home().join("decline"),
        apps_dir: dir.path().join("Applications"),
        exe: PathBuf::from(APP),
    };
    std::fs::create_dir_all(&paths.apps_dir).unwrap();
    let extra = dir.path().join("extra.toml");
    std::fs::write(&extra, "font_size = 17\n").unwrap();
    let script = format!("echo pinned {unique}; {}{}sleep 20", osc("dump"), osc("quit"));
    let options = Options {
        name: format!("Decline Smoke {unique}"),
        icon: None,
        config: Some(extra),
        cwd: None,
        dir: paths.apps_dir.clone(),
        dock: false,
        command: vec!["/bin/sh".into(), "-c".into(), script],
    };
    // The Dock is never touched; everything else runs for real.
    let mut runner = |cmd: &mut Command| {
        let program = cmd.get_program().to_string_lossy().into_owned();
        assert!(!program.ends_with("defaults") && !program.ends_with("killall"), "{program}");
        cmd.status()
    };
    let made = pin::create(options, &paths, &mut runner).expect("creating the pin");
    let id = format!("{}decline-smoke-{unique}", pin::ID_PREFIX);
    assert!(std::fs::read_to_string(made.app.join("Contents/Info.plist")).unwrap().contains(&id));

    let mut open = Command::new("/usr/bin/open");
    open.arg("-n")
        .arg("-W")
        .arg("--env")
        .arg(format!("DECLINE_TEST_DUMP={}", dir.dump_file().display()))
        .arg("--env")
        .arg(format!("XDG_CONFIG_HOME={}", dir.config_home().display()))
        .arg("--stderr")
        .arg(dir.path().join("stderr.txt"))
        .arg(&made.app);
    let (status, _) = run(&mut open);
    let lsregister = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    let _ = Command::new(lsregister).arg("-u").arg(&made.app).status();
    assert!(status.success(), "open: {status}");

    let d = dumps(&dir);
    let stderr = std::fs::read_to_string(dir.path().join("stderr.txt")).unwrap_or_default();
    assert_eq!(d.len(), 1, "dumps {d:?}; the app's stderr:\n{stderr}");
    check_dump(&d[0]);
    assert!(screen(&d[0]).starts_with(&format!("pinned {unique}|")), "{}", screen(&d[0]));
    assert_eq!(num(&d[0], "font_size"), 17.0, "the pin's own settings were used");
}
