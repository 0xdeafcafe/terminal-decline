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

/// A second tab through the `tab` control command: both tabs run their
/// program, the tab bar that appears costs both of them lines at the top,
/// the first tab's program is told its new size, and the first tab, now in
/// the background, is hidden and gives back its drawables; brought back to
/// the front (the `front` control command), it shows again.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_second_tab_shrinks_the_grid() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("tab");
    let d = dir.path().display();
    // The same program runs in both tabs; the first to make the directory
    // is the first tab.
    let wait = |cond: &str| format!("i=0; while ! {{ {cond}; }} && [ $i -lt 200 ]; do sleep 0.05; i=$((i+1)); done; ");
    let script = format!(
        "if mkdir '{d}/first' 2>/dev/null; then \
           s0=$(stty size); echo one; {dump}{tab}\
           {wait_two}{wait_size}stty size; {dump}{front}{dump}{quit}\
         else \
           echo two; {dump}{wait_dump}touch '{d}/two-done'; \
         fi; sleep 20",
        dump = osc("dump"),
        tab = osc("tab"),
        front = osc("front"),
        quit = osc("quit"),
        wait_two = wait(&format!("[ -e '{d}/two-done' ]")),
        wait_size = wait("[ \"$(stty size)\" != \"$s0\" ]"),
        wait_dump = wait(&format!("grep -q '\"window\":2' '{}'", dir.dump_file().display())),
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let all = dumps(&dir);
    for x in &all {
        check_dump(x);
    }
    let of = |w: u64| all.iter().filter(|x| x["window"] == w).collect::<Vec<_>>();
    let (one, two) = (of(1), of(2));
    assert_eq!((one.len(), two.len()), (3, 1), "{all:?}");
    let (before, after, back, two) = (one[0], one[1], one[2], two[0]);
    assert!(screen(before).starts_with("one|"), "{}", screen(before));
    assert!(screen(two).starts_with("two|"), "{}", screen(two));
    assert_eq!(num(before, "tabs"), 1.0, "{before}");
    assert_eq!((num(after, "tabs"), num(two, "tabs")), (2.0, 2.0), "{after} {two}");
    // The tab bar pushes the grid down in both tabs, and costs lines.
    assert!(num(after, "pad_top") > num(before, "pad_top"), "{before} {after}");
    assert_eq!(num(two, "pad_top"), num(after, "pad_top"), "{two} {after}");
    let lines = |x: &Value| num(x, "lines");
    assert!(lines(after) < lines(before), "{before} {after}");
    assert_eq!(lines(two), lines(after));
    assert_eq!(num(two, "cols"), num(after, "cols"));
    // The program in the first tab was told.
    let size = format!("{} {}|", lines(after), num(after, "cols"));
    assert!(screen(after).contains(&size), "{size} in {}", screen(after));
    // The first tab is behind the second: hidden, and it gave back memory.
    assert_eq!(after["visible"], false, "{after}");
    assert!(num(after, "hidden") >= 1.0, "{after}");
    // In front again: not hidden again, and, as the key window, shown at
    // once. (If the app isn't active, as when another test's app has just
    // quit, it has no key window: it is shown when macOS reports it
    // uncovered, which may come after this dump.)
    assert_eq!(num(back, "hidden"), num(after, "hidden"), "{back}");
    if back["active"] == true {
        assert_eq!(back["visible"], true, "{back}");
    }
}

/// What a dogfood build does after a restart (`--restore <file>`): two
/// tabs reopen as tabs of one window, in order, each in its directory, and
/// a plain window stays a window of its own.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_restore_reopens_tabs_as_tabs() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("restore");
    let sub = |name: &str| {
        let p = dir.path().join(name);
        std::fs::create_dir_all(&p).unwrap();
        p.display().to_string()
    };
    let (a, b, c) = (sub("tab-a"), sub("tab-b"), sub("solo"));
    let restore = dir.path().join("restore.json");
    let saved = serde_json::json!([
        {"cwd": a, "frame": [100.0, 100.0, 700.0, 500.0], "tab_group": 0},
        {"cwd": c, "frame": [200.0, 150.0, 700.0, 500.0]},
        {"cwd": b, "frame": [100.0, 100.0, 700.0, 500.0], "tab_group": 0, "selected": true},
    ]);
    std::fs::write(&restore, saved.to_string()).unwrap();
    // Each window says where it is, and the last to dump quits.
    let all_dumped = format!("[ \"$(wc -l < '{}')\" -ge 3 ]", dir.dump_file().display());
    let script = format!(
        "echo \"in $(basename \"$PWD\")\"; {}i=0; while ! {all_dumped} && [ $i -lt 200 ]; do sleep 0.05; i=$((i+1)); done; {}sleep 20",
        osc("dump"),
        osc("quit")
    );
    let mut cmd = Command::new(APP);
    cmd.arg("--restore")
        .arg(&restore)
        .args(["-e", "/bin/sh", "-c", &script])
        .env("XDG_CONFIG_HOME", dir.config_home())
        .env("DECLINE_TEST_DUMP", dir.dump_file())
        .current_dir(dir.path());
    let (status, _) = run(&mut cmd);
    assert!(status.success(), "{status}");
    assert!(!restore.exists(), "the restore file is used up");
    let all = dumps(&dir);
    let at = |name: &str| {
        all.iter()
            .find(|x| screen(x).starts_with(&format!("in {name}|")))
            .unwrap_or_else(|| panic!("no window in {name}: {all:?}"))
    };
    let place = |x: &Value| (num(x, "tabs") as u32, num(x, "tab") as u32);
    assert_eq!(place(at("tab-a")), (2, 0));
    assert_eq!(place(at("tab-b")), (2, 1));
    assert_eq!(place(at("solo")), (1, 0));
    // Opened in the order saved, the group where it was first seen.
    let id = |name| num(at(name), "window");
    assert!(id("tab-a") < id("tab-b") && id("tab-b") < id("solo"), "{all:?}");
}

/// The bare binary (no app bundle) can't use the notification center,
/// which would throw: OSC 9 and 777 fall back to the bell's Dock bounce,
/// and OSC 9;4 progress still shows on the Dock icon.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_notifications_without_a_bundle() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("notify");
    let script = notify_from_a_background_tab(&dir);
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "the app survived: {status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 2, "{d:?}");
    assert_eq!(d[0]["tabs"], 2, "{}", d[0]);
    let n = &d[0]["notifications"];
    assert_eq!(n["permission"], "unbundled", "{n}");
    assert_eq!(n["posted"], 0, "{n}");
    assert_eq!(n["bounced"], 2, "{n}");
    assert_eq!(d[0]["badge"], "42%", "{}", d[0]);
    assert_eq!(d[1]["badge"], Value::Null, "cleared: {}", d[1]);
}

/// The bundled app asks for permission on the first notification and
/// posts through the notification center. Opt in with
/// `DECLINE_SMOKE_NOTIFY=1`: each run's temp bundle leaves an entry in
/// System Settings > Notifications and may show a permission prompt.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_notifications_from_a_bundle() {
    if std::env::var_os("DECLINE_SMOKE_NOTIFY").is_none() {
        eprintln!("skipped: set DECLINE_SMOKE_NOTIFY=1 to run");
        return;
    }
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("notify-bundle");
    let unique = format!("{}", std::process::id());
    let app = temp_bundle(&dir, &format!("Decline Notify {unique}"), &notify_from_a_background_tab(&dir));
    let d = launch_bundle(&dir, &app);
    assert_eq!(d.len(), 2, "{d:?}");
    let n = &d[0]["notifications"];
    eprintln!("notifications: {n}");
    // Asked once; the two wait for the answer, or were posted, or (turned
    // off) bounced the Dock icon.
    assert!(["asking", "granted", "denied"].contains(&n["permission"].as_str().unwrap()), "{n}");
    assert_eq!(num(n, "posted") + num(n, "bounced"), 2.0, "{n}");
    assert_eq!(d[0]["badge"], "42%", "{}", d[0]);
}

/// A program for two tabs: the first opens the second and, from behind it
/// (where a notification is always wanted), asks for two notifications and
/// reports progress, dumps, clears the progress, dumps and quits. Output
/// after a dump is parsed while the dump waits for its frame, so the
/// progress is cleared only once the first dump is written.
fn notify_from_a_background_tab(dir: &TempDir) -> String {
    let d = dir.path().display();
    let wait = |cond: String| format!("i=0; while ! {{ {cond}; }} && [ $i -lt 200 ]; do sleep 0.05; i=$((i+1)); done; ");
    format!(
        "if mkdir '{d}/first' 2>/dev/null; then \
           {tab}{wait_two}\
           printf '\\033]9;Claude Code: done\\007'; sleep 0.2; \
           printf '\\033]777;notify;Build;finished\\007\\033]9;4;1;42\\007'; echo sent; {dump}\
           {wait_dump}printf '\\033]9;4;0;\\007'; {dump}{quit}\
         else touch '{d}/two'; fi; sleep 20",
        tab = osc("tab"),
        wait_two = wait(format!("[ -e '{d}/two' ]")),
        dump = osc("dump"),
        wait_dump = wait(format!("[ -s '{}' ]", dir.dump_file().display())),
        quit = osc("quit"),
    )
}

/// Secure Keyboard Entry saved as on in the app's user defaults (a temp
/// bundle has a domain of its own): held while the app is active.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_secure_keyboard_entry_from_user_defaults() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("secure");
    let unique = format!("{}", std::process::id());
    let id = format!("{}decline-secure-{unique}", decline::pin::ID_PREFIX);
    /// Removes the temp bundle's defaults domain, even if the test fails.
    struct Domain(String);
    impl Drop for Domain {
        fn drop(&mut self) {
            let _ = Command::new("/usr/bin/defaults").args(["delete", &self.0]).status();
        }
    }
    let status = Command::new("/usr/bin/defaults")
        .args(["write", &id, "SecureKeyboardEntry", "-bool", "true"])
        .status()
        .unwrap();
    let _domain = Domain(id.clone());
    assert!(status.success());
    let script = format!("echo secure; sleep 0.3; {}{}sleep 20", osc("dump"), osc("quit"));
    let app = temp_bundle(&dir, &format!("Decline Secure {unique}"), &script);
    let d = launch_bundle(&dir, &app);
    assert_eq!(d.len(), 1, "{d:?}");
    // macOS may refuse to activate an app started while someone types in
    // another; then it must not hold secure input.
    assert_eq!(d[0]["secure_input"], d[0]["active"], "{}", d[0]);
    if d[0]["active"] == false {
        eprintln!("the app wasn't activated: only checked that it doesn't hold secure input");
    }
}

/// A pin bundle for `script` in `dir`'s Applications, made by the pin
/// module the way `decline pin` does (the Dock is never touched).
fn temp_bundle(dir: &TempDir, name: &str, script: &str) -> PathBuf {
    use decline::pin::{self, Options};
    let paths = decline::Paths {
        config_dir: dir.config_home().join("decline"),
        apps_dir: dir.path().join("Applications"),
        exe: PathBuf::from(APP),
    };
    std::fs::create_dir_all(&paths.apps_dir).unwrap();
    let options = Options {
        name: name.into(),
        icon: None,
        config: None,
        cwd: None,
        dir: paths.apps_dir.clone(),
        dock: false,
        command: vec!["/bin/sh".into(), "-c".into(), script.into()],
    };
    let mut runner = |cmd: &mut Command| {
        let program = cmd.get_program().to_string_lossy().into_owned();
        assert!(!program.ends_with("defaults") && !program.ends_with("killall"), "{program}");
        cmd.status()
    };
    pin::create(options, &paths, &mut runner).expect("creating the pin").app
}

/// Launch `app` as the Dock does and wait for it to quit; its dumps.
fn launch_bundle(dir: &TempDir, app: &Path) -> Vec<Value> {
    let mut open = Command::new("/usr/bin/open");
    open.arg("-n")
        .arg("-W")
        .arg("--env")
        .arg(format!("DECLINE_TEST_DUMP={}", dir.dump_file().display()))
        .arg("--env")
        .arg(format!("XDG_CONFIG_HOME={}", dir.config_home().display()))
        .arg("--stderr")
        .arg(dir.path().join("stderr.txt"))
        .arg(app);
    let (status, _) = run(&mut open);
    let lsregister = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    let _ = Command::new(lsregister).arg("-u").arg(app).status();
    let stderr = std::fs::read_to_string(dir.path().join("stderr.txt")).unwrap_or_default();
    assert!(status.success(), "open: {status}; the app's stderr:\n{stderr}");
    dumps(dir)
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

/// Launch `decline <args>` with the dump file, for a program that fails at
/// once. The window then stays open and (under test control) dumps what it
/// shows: return that dump once the app has stayed running past it, then
/// kill the app.
fn held_window(dir: &TempDir, args: &[&str]) -> Value {
    let mut cmd = Command::new(APP);
    cmd.args(args)
        .env("XDG_CONFIG_HOME", dir.config_home())
        .env("DECLINE_TEST_DUMP", dir.dump_file())
        .env("SHELL", "/bin/sh")
        .current_dir(dir.path());
    let mut child = cmd.spawn().unwrap_or_else(|e| panic!("launching {cmd:?}: {e}"));
    let start = Instant::now();
    let dump = loop {
        if let Some(d) = dumps(dir).into_iter().next() {
            break d;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("the app exited ({status}) instead of keeping the window open");
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            panic!("no dump after {TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    std::thread::sleep(Duration::from_millis(500));
    let still_running = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let _ = child.wait();
    assert!(still_running, "the window closed by itself");
    dump
}

#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_quick_failure_keeps_the_window() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("exit3");
    let d = held_window(&dir, &["-e", "/bin/sh", "-c", "echo failing; exit 3"]);
    check_dump(&d);
    let s = screen(&d);
    assert!(
        s.starts_with("failing|\n|\n[process exited with status 3]|\npress any key to close"),
        "{s}"
    );
}

#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_missing_program_keeps_the_window() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("missing");
    let d = held_window(&dir, &["-e", "/nonexistent"]);
    let s = screen(&d);
    assert!(s.contains("/nonexistent"), "{s}");
    assert!(s.contains("[process exited with status 12"), "{s}");
    assert!(s.contains("press any key to close"), "{s}");
}

#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_quick_success_closes_the_window() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("exit0");
    let (status, _) = decline(&dir, "exit 0", true);
    assert!(status.success(), "{status}");
    assert!(dumps(&dir).is_empty());
}

#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_config_errors_show_in_the_window() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("badconfig");
    std::fs::create_dir_all(dir.config_home().join("decline")).unwrap();
    std::fs::write(dir.config_home().join("decline/config.toml"), "font_sise = 3\n").unwrap();
    let script = format!("echo program output; {}{}sleep 20", osc("dump"), osc("quit"));
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 1, "{d:?}");
    let s = screen(&d[0]);
    assert!(s.starts_with("decline: config: font_sise: unknown field"), "{s}");
    assert!(s.contains("ignored|\nprogram output|"), "{s}");
}

/// Font settings reach the window: `line_height = 1.5` makes cells half
/// as tall again as the default config's, keeps doing so after a font
/// size change, and a fallback font that isn't installed is reported in
/// the window.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_font_settings_from_the_config() {
    if !have_metal() {
        return;
    }
    let script = format!("echo text; {}{}{}{}sleep 20", osc("dump"), osc("font +1"), osc("dump"), osc("quit"));
    let run_with = |name: &str, config: &str| {
        let dir = TempDir::new(name);
        std::fs::create_dir_all(dir.config_home().join("decline")).unwrap();
        std::fs::write(dir.config_home().join("decline/config.toml"), config).unwrap();
        let (status, _) = decline(&dir, &script, true);
        assert!(status.success(), "{status}");
        let d = dumps(&dir);
        assert_eq!(d.len(), 2, "{d:?}");
        check_dump(&d[0]);
        d
    };
    let plain = run_with("font-default", "");
    let tall = run_with("font-tall", "line_height = 1.5\nfont_fallback = ['No Such Font For Decline']\n");
    assert_eq!(num(&plain[0], "scale"), num(&tall[0], "scale"));
    for (p, t) in plain.iter().zip(&tall) {
        let (hp, ht) = (num(p, "cell_height"), num(t, "cell_height"));
        assert!((ht - hp * 1.5).abs() <= 1.0, "{hp} vs {ht}");
        assert_eq!(num(p, "cell_width"), num(t, "cell_width"));
    }
    let s = screen(&tall[0]);
    assert!(
        s.starts_with("decline: config: font_fallback: no font named \"No Such Font For Decline\" is installed; ignored|"),
        "{s}"
    );
    assert!(screen(&plain[0]).starts_with("text|"), "{}", screen(&plain[0]));
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
