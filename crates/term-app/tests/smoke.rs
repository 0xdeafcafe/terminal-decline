//! Smoke tests of the real app: they open windows, so they are `#[ignore]`d
//! and run one at a time (nextest group `gui`). Run them with
//! `cargo nextest run -p terminal-decline --run-ignored only -E 'binary(smoke)'`.
//!
//! Each launches the built `terminal-decline` with
//! `DECLINE_TEST_DUMP=<file>`, so a program in the window can drive it with
//! the private control OSC (see `terminal_decline_app::control`): `dump` writes a
//! line of JSON after the next frame the GPU has finished, then `resize`,
//! `font` and `quit`. The tests assert on the dumps, not on pixels. Config
//! comes from an empty temp dir (`XDG_CONFIG_HOME`), never the user's.
//!
//! Under the test control the app stays in the background (see
//! `background.rs`): it never activates and its windows, transparent, sit
//! behind every other window. Bundles are opened with `open -g` so
//! Launch Services doesn't bring them to the front either. Set
//! `DECLINE_TEST_FOREGROUND=1` to watch a run.
//!
//! On CI (`CI` set) a missing Metal device fails the test; locally it is
//! skipped. A launch failure always fails.
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

use serde_json::Value;

mod support;

const APP: &str = env!("CARGO_BIN_EXE_terminal-decline");
/// Longest a scenario may take before the app is killed.
const TIMEOUT: Duration = Duration::from_secs(30);

/// `printf` arguments for a control command.
fn osc(cmd: &str) -> String {
    format!("printf '\\033]7770;{cmd}\\007'; ")
}

/// Shell that waits until this script's process group owns the terminal.
/// `set -m` forks the job and hands it the terminal a moment later, and
/// closing the window before that finds only the shell running. The ids
/// are compared as numbers: `ps` pads TPGID and PGID to different widths,
/// so with pids under 10000 (as on a freshly booted CI runner) the strings
/// never match.
fn in_foreground() -> &'static str {
    "until [ \"$(ps -o tpgid= -p $$)\" -eq \"$(ps -o pgid= -p $$)\" ]; do sleep 0.05; done; "
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("terminal-decline-smoke-{}-{name}", std::process::id()));
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

/// `terminal-decline -e sh -c <script>` with the dump file, a temp config
/// home and any `env`.
fn decline(dir: &TempDir, script: &str, dump: bool) -> (ExitStatus, Duration) {
    decline_from(Path::new(APP), dir, script, dump)
}

/// [`decline`] with another executable: the bundled app's launcher.
fn decline_from(program: &Path, dir: &TempDir, script: &str, dump: bool) -> (ExitStatus, Duration) {
    let mut cmd = Command::new(program);
    cmd.args(["-e", "/bin/sh", "-c", script])
        .env("XDG_CONFIG_HOME", dir.config_home())
        .env_remove("DECLINE_TEST_DUMP")
        .current_dir(dir.path());
    if dump {
        cmd.env("DECLINE_TEST_DUMP", dir.dump_file());
    } else {
        // In the background all the same (the test control does that).
        cmd.env("DECLINE_TEST_BACKGROUND", "1");
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

/// The bare binary never runs the updater: `update-check` does nothing,
/// and the dump says why it is off (docs/design/updater.md §2).
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_updater_is_off_outside_the_app() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("update");
    let script = format!("{}{}{}sleep 20", osc("update-check"), osc("dump"), osc("quit"));
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0]["update"], "off: not Terminal Decline.app (the bare binary)", "{}", d[0]);
}

/// A pin never loads Sparkle: its dump says the updater is off as a pin.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_updater_is_off_in_a_pin() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("update-pin");
    let unique = format!("{}", std::process::id());
    let script = format!("echo pin; sleep 0.3; {}{}{}sleep 20", osc("update-check"), osc("dump"), osc("quit"));
    let app = temp_bundle(&dir, &format!("Terminal Decline Update {unique}"), &script);
    let d = launch_bundle(&dir, &app);
    assert_eq!(d.len(), 1, "{d:?}");
    let update = d[0]["update"].as_str().unwrap_or_default();
    assert!(update.starts_with("off: "), "{}", d[0]);
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

/// The app as it ships (docs/design/pins.md §2): `scripts/bundle.sh`'s
/// bundle, hardened, its launcher loading the library, which opens a
/// window and draws.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_bundled_app_draws() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("bundled");
    let app = support::bundle(&dir.path().join("out"), &[("HARDENED", "1")]);
    let script = format!("echo 'hello from the bundle'; {}{}sleep 20", osc("dump"), osc("quit"));
    let (status, _) = decline_from(&support::launcher(&app), &dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 1, "{d:?}");
    check_dump(&d[0]);
    assert!(screen(&d[0]).starts_with("hello from the bundle|\n"), "{}", screen(&d[0]));
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
    // Quietly: until the first dump the file is missing, and error lines
    // from each try would scroll the first line out of a dump taken late.
    let all_dumped = format!("[ \"$(cat '{}' 2>/dev/null | wc -l)\" -ge 3 ]", dir.dump_file().display());
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

/// A restored tab whose shell was running a program gets its command line
/// typed at the new shell's prompt, not run. Needs zsh as the login shell
/// (its startup files come from a temp `ZDOTDIR`): once the command is in
/// zsh's line, `zle-line-pre-redraw` asks for a dump and then quits.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_restore_types_the_command_at_the_prompt() {
    if !have_metal() {
        return;
    }
    if !term_pty::login_shell().ends_with("/zsh") {
        eprintln!("skipped: the login shell isn't zsh");
        return;
    }
    let dir = TempDir::new("restore-command");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let zshrc = format!(
        r#"PS1='READY> '
td_seen=
td-line-pre-redraw() {{
  [[ -n $BUFFER && -z $td_seen ]] || return 0
  td_seen=1
  {{ sleep 0.5; {dump}i=0; while [ ! -s "$DECLINE_TEST_DUMP" ] && [ $i -lt 200 ]; do sleep 0.05; i=$((i+1)); done; {quit}}} > /dev/tty 2>&1 &!
}}
zle -N zle-line-pre-redraw td-line-pre-redraw
"#,
        dump = osc("dump"),
        quit = osc("quit"),
    );
    std::fs::write(dir.path().join(".zshrc"), zshrc).unwrap();
    let restore = dir.path().join("restore.json");
    let saved = serde_json::json!([
        {"cwd": work, "frame": [100.0, 100.0, 700.0, 500.0], "command": "touch 'ran here'"},
    ]);
    std::fs::write(&restore, saved.to_string()).unwrap();
    let mut cmd = Command::new(APP);
    cmd.arg("--restore")
        .arg(&restore)
        .env("XDG_CONFIG_HOME", dir.config_home())
        .env("ZDOTDIR", dir.path())
        .env("DECLINE_TEST_DUMP", dir.dump_file())
        .current_dir(dir.path());
    let (status, _) = run(&mut cmd);
    assert!(status.success(), "{status}");
    let all = dumps(&dir);
    assert_eq!(all.len(), 1, "{all:?}");
    let shown = screen(&all[0]);
    assert!(shown.starts_with("READY> touch 'ran here'|"), "{shown}");
    assert!(!work.join("ran here").exists(), "it ran without Enter");
}

/// Under the test control notifications go to the dump, not the
/// notification center: OSC 9 and 777 from a background tab arrive as
/// one notification (the second replaces the first, as a burst from one
/// window does), OSC 9;4 progress shows on the Dock icon, and once it is
/// cleared the badge counts the notification not yet looked at.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_notifications_go_to_the_dump() {
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
    assert_eq!(n["permission"], "test", "{n}");
    assert_eq!(n["posted"], 2, "{n}");
    assert_eq!(n["bounced"], 0, "{n}");
    let sent = n["sent"].as_array().unwrap();
    assert_eq!(sent[0]["body"], "Claude Code: done", "{n}");
    assert_eq!(sent[1]["title"], "Build", "{n}");
    assert_eq!(sent[1]["body"], "finished · and 1 more", "{n}");
    assert_eq!(sent[0]["key"], sent[1]["key"], "one burst: {n}");
    assert!(sent[0]["target"]["pane"].is_object(), "{n}");
    assert_eq!(d[0]["badge"], "42%", "{}", d[0]);
    assert_eq!(d[1]["badge"], "1", "the unseen notification: {}", d[1]);
}

/// A command (marked with OSC 133, as the shell integration does) that
/// runs past `notify_after` in a background tab is a notification when
/// it finishes, with its exit status; a bell from there is one too.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_a_long_command_finishing_in_a_background_tab_notifies() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("notify-finish");
    let config = dir.config_home().join("terminal-decline");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("config.toml"), "notify_after = 1\n").unwrap();
    let d = dir.path().display();
    let wait = |cond: String| format!("i=0; while ! {{ {cond}; }} && [ $i -lt 200 ]; do sleep 0.05; i=$((i+1)); done; ");
    let script = format!(
        "if mkdir '{d}/first' 2>/dev/null; then {tab}{wait_two}\
         printf '\\033]133;A\\007$ \\033]133;B\\007make test\\r\\n\\033]133;C\\007'; sleep 1.2; \
         printf '\\033]133;D;3\\007'; \
         printf '\\033]133;A\\007$ \\033]133;B\\007true\\r\\n\\033]133;C\\007\\033]133;D;0\\007'; \
         sleep 11; printf '\\007'; sleep 0.2; {dump}{quit}\
         else touch '{d}/two'; fi; sleep 30",
        tab = osc("tab"),
        wait_two = wait(format!("[ -e '{d}/two' ]")),
        dump = osc("dump"),
        quit = osc("quit"),
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let all = dumps(&dir);
    let n = &all[0]["notifications"];
    let sent = n["sent"].as_array().unwrap();
    // The quick `true` isn't one; the bell, after the burst, is its own.
    assert_eq!(sent.len(), 2, "{n}");
    assert_eq!(sent[0]["title"], "✗ make test — exit 3", "{n}");
    assert!(sent[0]["body"].as_str().unwrap().ends_with("after 1s"), "{n}");
    assert!(sent[1]["body"].as_str().unwrap().starts_with("Bell"), "{n}");
    assert_ne!(sent[0]["key"], sent[1]["key"], "{n}");
    assert_eq!(all[0]["badge"], "2", "{}", all[0]);
}

/// With the shell's OSC 133 marks the title says what it does: the
/// command running and for how long (the clock ticks), then waiting in
/// its folder.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_the_title_says_what_the_shell_does() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("title-status");
    let script = format!(
        "printf '\\033]133;A\\007$ \\033]133;B\\007make all\\r\\n\\033]133;C\\007'; sleep 2.3; {dump}\
         sleep 0.5; printf '\\033]133;D;0\\007\\033]133;A\\007$ '; sleep 0.3; {dump}{quit}sleep 10",
        dump = osc("dump"),
        quit = osc("quit"),
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 2, "{d:?}");
    assert_eq!(d[0]["title"], "make all · 2s — Terminal Decline", "{}", d[0]);
    let waiting = d[1]["title"].as_str().unwrap();
    assert!(
        waiting.starts_with("waiting · ") && waiting.ends_with(" — Terminal Decline"),
        "{waiting}"
    );
}

/// The login shell gets the shell integration without sourcing anything
/// (`shell_integration` is on by default): a long command typed at a
/// plain zsh's prompt puts the title status up, then waiting. Needs zsh
/// as the login shell; its startup files come from a temp `ZDOTDIR`,
/// whose `.zshrc` sets only a prompt and, from `zle-line-init`, runs a
/// command at the first prompt and dumps and quits at the second.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_the_login_shell_gets_the_integration_by_itself() {
    if !have_metal() {
        return;
    }
    if !term_pty::login_shell().ends_with("/zsh") {
        eprintln!("skipped: the login shell isn't zsh");
        return;
    }
    let dir = TempDir::new("integration-login");
    let zshrc = format!(
        r#"PS1='READY> '
td_prompts=0
td_run() {{ sleep 2.3; {dump}sleep 0.3; }}
td-line-init() {{
  td_prompts=$((td_prompts + 1))
  if (( td_prompts == 1 )); then
    BUFFER=td_run
    zle accept-line
  elif (( td_prompts == 2 )); then
    {{ sleep 0.5; {dump}sleep 0.5; {quit}}} > /dev/tty 2>&1 &!
  fi
}}
zle -N zle-line-init td-line-init
"#,
        dump = osc("dump"),
        quit = osc("quit"),
    );
    std::fs::write(dir.path().join(".zshrc"), zshrc).unwrap();
    let mut cmd = Command::new(APP);
    cmd.env("XDG_CONFIG_HOME", dir.config_home())
        .env("ZDOTDIR", dir.path())
        .env("DECLINE_TEST_DUMP", dir.dump_file())
        .current_dir(dir.path());
    let (status, _) = run(&mut cmd);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 2, "{d:?}");
    assert_eq!(d[0]["title"], "td_run · 2s — Terminal Decline", "{}", d[0]);
    let waiting = d[1]["title"].as_str().unwrap();
    assert!(
        waiting.starts_with("waiting · ") && waiting.ends_with(" — Terminal Decline"),
        "{waiting}"
    );
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
    let app = temp_bundle(
        &dir,
        &format!("Terminal Decline Notify {unique}"),
        &notify_from_a_background_tab(&dir),
    );
    let d = launch_bundle_with(&dir, &app, &["DECLINE_TEST_REAL_NOTIFY=1"]);
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
    let id = format!("{}terminal-decline-secure-{unique}", terminal_decline_app::pin::ID_PREFIX);
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
    let app = temp_bundle(&dir, &format!("Terminal Decline Secure {unique}"), &script);
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
/// module the way `terminal-decline pin` does (the Dock is never touched).
fn temp_bundle(dir: &TempDir, name: &str, script: &str) -> PathBuf {
    use terminal_decline_app::pin::{self, Options};
    let paths = terminal_decline_app::Paths {
        config_dir: dir.config_home().join("terminal-decline"),
        apps_dir: dir.path().join("Applications"),
        exe: PathBuf::from(APP),
        app_bundle: None,
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
    launch_bundle_with(dir, app, &[])
}

/// [`launch_bundle`] with more environment (`NAME=value`).
fn launch_bundle_with(dir: &TempDir, app: &Path, env: &[&str]) -> Vec<Value> {
    let mut open = Command::new("/usr/bin/open");
    open.args(["-g", "-n", "-W"]);
    for e in env {
        open.arg("--env").arg(e);
    }
    open.arg("--env")
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

/// Launch `terminal-decline <args>` with the dump file, for a program that
/// fails at once. The window then stays open and (under test control) dumps
/// what it shows: return that dump once the app has stayed running past it,
/// then kill the app.
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
    std::fs::create_dir_all(dir.config_home().join("terminal-decline")).unwrap();
    std::fs::write(dir.config_home().join("terminal-decline/config.toml"), "font_sise = 3\n").unwrap();
    let script = format!("echo program output; {}{}sleep 20", osc("dump"), osc("quit"));
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 1, "{d:?}");
    let s = screen(&d[0]);
    assert!(s.starts_with("terminal-decline: config: font_sise: unknown field"), "{s}");
    // Joined where the long notice wrapped (⏎); spaces are left out, as
    // a wrap may fall on one (which the dump shows as ·).
    let joined: String = s.replace("⏎\n", "").chars().filter(|c| !matches!(c, ' ' | '·')).collect();
    assert!(joined.contains(";ignored|\nprogramoutput|"), "{s}");
}

/// The first run after the rename moves `~/.config/decline` to
/// `~/.config/terminal-decline` and reads the config from there.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_old_config_dir_is_moved_and_read() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("oldconfig");
    let old = dir.config_home().join("decline");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("config.toml"), "font_sise = 3\n").unwrap();
    let script = format!("{}{}sleep 20", osc("dump"), osc("quit"));
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 1, "{d:?}");
    let s = screen(&d[0]);
    assert!(s.starts_with("terminal-decline: config: moved "), "{s}");
    assert!(s.contains("terminal-decline: config: font_sise: unknown field"), "{s}");
    let new = dir.config_home().join("terminal-decline");
    assert_eq!(std::fs::read_to_string(new.join("config.toml")).unwrap(), "font_sise = 3\n");
    assert_eq!(std::fs::read_link(&old).unwrap(), Path::new("terminal-decline"));
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
        std::fs::create_dir_all(dir.config_home().join("terminal-decline")).unwrap();
        std::fs::write(dir.config_home().join("terminal-decline/config.toml"), config).unwrap();
        let (status, _) = decline(&dir, &script, true);
        assert!(status.success(), "{status}");
        let d = dumps(&dir);
        assert_eq!(d.len(), 2, "{d:?}");
        check_dump(&d[0]);
        d
    };
    let plain = run_with("font-default", "");
    let tall = run_with("font-tall", "line_height = 1.5\nfont_fallback = ['No Such Font Here']\n");
    assert_eq!(num(&plain[0], "scale"), num(&tall[0], "scale"));
    for (p, t) in plain.iter().zip(&tall) {
        let (hp, ht) = (num(p, "cell_height"), num(t, "cell_height"));
        assert!((ht - hp * 1.5).abs() <= 1.0, "{hp} vs {ht}");
        assert_eq!(num(p, "cell_width"), num(t, "cell_width"));
    }
    let s = screen(&tall[0]);
    assert!(
        s.starts_with("terminal-decline: config: font_fallback: no font named \"No Such Font Here\" is installed; ignored|"),
        "{s}"
    );
    assert!(screen(&plain[0]).starts_with("text|"), "{}", screen(&plain[0]));
}

/// A pin bundle made by the pin module in a temp dir, with a unique bundle
/// id, launched the way the Dock does (LaunchServices, via `open`).
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_pin_bundle_launches() {
    use terminal_decline_app::pin::{self, Options};
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
    let paths = terminal_decline_app::Paths {
        config_dir: dir.config_home().join("terminal-decline"),
        apps_dir: dir.path().join("Applications"),
        exe: PathBuf::from(APP),
        app_bundle: None,
    };
    std::fs::create_dir_all(&paths.apps_dir).unwrap();
    let extra = dir.path().join("extra.toml");
    std::fs::write(&extra, "font_size = 17\n").unwrap();
    let script = format!("echo pinned {unique}; {}{}sleep 20", osc("dump"), osc("quit"));
    let options = Options {
        name: format!("Terminal Decline Smoke {unique}"),
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
    let id = format!("{}terminal-decline-smoke-{unique}", pin::ID_PREFIX);
    assert!(std::fs::read_to_string(made.app.join("Contents/Info.plist")).unwrap().contains(&id));

    let mut open = Command::new("/usr/bin/open");
    open.args(["-g", "-n", "-W"])
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

/// A stub pin (pins.md §3), made from the bundled app by the pin module as
/// `terminal-decline pin` does there, launched the way the Dock does: its
/// stub finds the app through `$HOME/.config/terminal-decline/pin-app`
/// (a temp `HOME`), checks and loads its library, and the pin's settings
/// are used.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_stub_pin_launches() {
    use terminal_decline_app::pin::{self, Options};
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("stub-pin");
    let app = support::bundle(&dir.path().join("out"), &[]);
    let unique = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );
    let paths = terminal_decline_app::Paths {
        config_dir: dir.config_home().join("terminal-decline"),
        apps_dir: dir.path().join("Applications"),
        exe: support::launcher(&app),
        app_bundle: Some(app.clone()),
    };
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join(".config/terminal-decline")).unwrap();
    std::fs::write(home.join(".config/terminal-decline/pin-app"), format!("{}\n", app.display())).unwrap();
    let extra = dir.path().join("extra.toml");
    std::fs::write(&extra, "font_size = 17\n").unwrap();
    let script = format!("echo stub pinned {unique}; {}{}sleep 20", osc("dump"), osc("quit"));
    let options = Options {
        name: format!("Terminal Decline Stub {unique}"),
        icon: None,
        config: Some(extra),
        cwd: None,
        dir: paths.apps_dir.clone(),
        dock: false,
        command: vec!["/bin/sh".into(), "-c".into(), script],
    };
    let mut runner = |cmd: &mut Command| {
        let program = cmd.get_program().to_string_lossy().into_owned();
        assert!(!program.ends_with("defaults") && !program.ends_with("killall"), "{program}");
        pin::system_runner(cmd)
    };
    let made = pin::create(options, &paths, &mut runner).expect("creating the pin");
    let plist = std::fs::read_to_string(made.app.join("Contents/Info.plist")).unwrap();
    assert!(plist.contains("<key>TDPinAbi</key>"), "{plist}");

    let mut open = Command::new("/usr/bin/open");
    open.args(["-g", "-n", "-W"])
        .arg("--env")
        .arg(format!("DECLINE_TEST_DUMP={}", dir.dump_file().display()))
        .arg("--env")
        .arg(format!("XDG_CONFIG_HOME={}", dir.config_home().display()))
        .arg("--env")
        .arg(format!("HOME={}", home.display()))
        .arg("--env")
        .arg("TD_PIN_NO_ALERT=1")
        .arg("--stderr")
        .arg(dir.path().join("stderr.txt"))
        .arg(&made.app);
    let (status, _) = run(&mut open);
    let lsregister = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    let _ = Command::new(lsregister).arg("-u").arg(&made.app).status();
    let stderr = std::fs::read_to_string(dir.path().join("stderr.txt")).unwrap_or_default();
    assert!(status.success(), "open: {status}; stderr:\n{stderr}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 1, "dumps {d:?}; the pin's stderr:\n{stderr}");
    check_dump(&d[0]);
    assert!(screen(&d[0]).starts_with(&format!("stub pinned {unique}|")), "{}", screen(&d[0]));
    assert_eq!(num(&d[0], "font_size"), 17.0, "the pin's own settings were used");
}

/// Themes: one named in the config but not there yet is a notice, and is
/// used once its file is created (theme files are watched too); a
/// light/dark pair follows the system's appearance.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_themes_load_and_follow_the_appearance() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("theme");
    let conf_dir = dir.config_home().join("terminal-decline");
    std::fs::create_dir_all(&conf_dir).unwrap();
    let file = conf_dir.join("config.toml");
    std::fs::write(&file, "theme = 'mine'\n").unwrap();
    let theme = conf_dir.join("themes/mine.toml");
    let script = format!(
        "echo start; {dump}{wait1}\
         mkdir -p '{dir}'; printf \"background = '#203040'\\n\" > '{theme}'; {wait2}\
         printf \"theme = {{ light = 'slope-light', dark = 'slope-dark' }}\\n\" > '{file}'; {wait3}{quit}sleep 20",
        dump = osc("dump"),
        dir = theme.parent().unwrap().display(),
        theme = theme.display(),
        file = file.display(),
        wait1 = dump_until(&dir, "\"seq\":1"),
        wait2 = dump_until(&dir, "#203040"),
        wait3 = dump_until(&dir, "#(092d2b|f3f3eb)"),
        quit = osc("quit"),
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert!(d.len() >= 3, "{d:?}");
    assert_eq!(d[0]["background"], "#16181c", "the defaults until the theme exists");
    assert!(
        screen(&d[0]).starts_with("terminal-decline: config: theme: no theme named \"mine\""),
        "{}",
        screen(&d[0])
    );
    assert!(
        d.iter().any(|x| x["background"] == "#203040" && x["foreground"] == "#d8dee9"),
        "{d:?}"
    );
    let dark = Command::new("/usr/bin/defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "Dark");
    let last = d.last().unwrap();
    let (fg, bg) = if dark { ("#f3f3eb", "#092d2b") } else { ("#092d2b", "#f3f3eb") };
    assert_eq!(
        (last["foreground"].as_str(), last["background"].as_str()),
        (Some(fg), Some(bg)),
        "dark: {dark}"
    );
}

/// A shell loop that sends `dump` every 0.1 s (for at most 10 s) until the
/// dump file matches `pattern` (grep -E).
fn dump_until(dir: &TempDir, pattern: &str) -> String {
    format!(
        "i=0; until grep -Eq '{pattern}' '{dump}' || [ $i -ge 100 ]; do {d}sleep 0.1; i=$((i+1)); done; ",
        dump = dir.dump_file().display(),
        d = osc("dump"),
    )
}

/// Editing the config while the window is open: a new background is drawn
/// (written the way editors save, a new file renamed over the old), and an
/// edit with a mistake becomes a notice in the window, once; that a
/// setting waits for new windows isn't written into it.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_config_reloads_when_edited() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("reload");
    let conf_dir = dir.config_home().join("terminal-decline");
    std::fs::create_dir_all(&conf_dir).unwrap();
    let file = conf_dir.join("config.toml");
    std::fs::write(&file, "[colors]\nbackground = '#000000'\n").unwrap();
    let f = file.display();
    let script = format!(
        "echo before; {dump}{wait1}\
         printf \"[colors]\\nbackground = '#102030'\\n\" > '{f}.tmp'; mv '{f}.tmp' '{f}'; \
         {wait2}\
         printf \"font_sise = 3\\ncolumns = 50\\n[colors]\\nbackground = '#102030'\\n\" > '{f}'; \
         {wait3}printf \"font_sise = 3\\ncolumns = 60\\n[colors]\\nbackground = '#102030'\\n\" > '{f}'; \
         sleep 1; {dump}{quit}sleep 20",
        dump = osc("dump"),
        wait1 = dump_until(&dir, "\"seq\":1"),
        wait2 = dump_until(&dir, "#102030"),
        wait3 = dump_until(&dir, "font_sise"),
        quit = osc("quit"),
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert!(d.len() >= 3, "{d:?}");
    check_dump(&d[0]);
    assert_eq!(d[0]["background"], "#000000", "{}", d[0]);
    assert_eq!(d[0]["foreground"], "#d8dee9", "{}", d[0]);
    let recoloured = d
        .iter()
        .position(|x| x["background"] == "#102030")
        .expect("the new colour was never drawn");
    assert!(recoloured > 0);
    let last = d.last().unwrap();
    assert_eq!(last["background"], "#102030", "{last}");
    let s = screen(last);
    assert!(s.starts_with("before|"), "{s}");
    assert_eq!(s.matches("terminal-decline: config: font_sise: unknown field").count(), 1, "{s}");
    assert!(!s.contains("new windows"), "{s}");
    assert!(
        !screen(&d[recoloured]).contains("terminal-decline:"),
        "a good edit shows nothing: {}",
        screen(&d[recoloured])
    );
}

/// Settings… (the `settings` control command, as Cmd+, does): a tab
/// running `terminal-decline prefs`, titled Settings, beside the window; a
/// second Cmd+, brings the same tab back rather than opening another. An
/// edit made in it recolours the other tab at once, and no notice is
/// written into that tab while Settings is open, even for a mistake made
/// by hand meanwhile. The prefs program's footprint and its idle wakeups
/// are printed.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_settings_opens_one_tab_that_edits_the_config() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("settings");
    let conf_dir = dir.config_home().join("terminal-decline");
    std::fs::create_dir_all(&conf_dir).unwrap();
    let file = conf_dir.join("config.toml");
    std::fs::write(&file, "# mine\n[colors]\nbackground = '#000000'\n").unwrap();
    let (f, d) = (file.display(), dir.path().display());
    // `[ ]` so the pattern doesn't match this script's own command line.
    let prefs = "pgrep -f 'terminal-decline[ ]prefs --layer'";
    let wait_prefs = format!("i=0; until {prefs} > '{d}/pid' || [ $i -ge 200 ]; do sleep 0.05; i=$((i+1)); done; sleep 1; ");
    let script = format!(
        "echo before; {dump}{wait1}{settings}{wait_prefs}{front}{settings}sleep 0.5; echo focused; {dump}{wait2}\
         {edit}{wait3}\
         {{ printf 'font_sise = 3\\n'; cat '{f}'; }} > '{f}.tmp'; mv '{f}.tmp' '{f}'; sleep 1; \
         p=$(cat '{d}/pid'); /usr/bin/footprint -p $p > '{d}/footprint.txt' 2>&1; \
         top -l 2 -s 3 -pid $p -stats pid,idlew,cpu,th > '{d}/top.txt' 2>&1; \
         echo edited; {dump}{wait4}{quit}sleep 20",
        dump = osc("dump"),
        settings = osc("settings"),
        front = osc("front"),
        edit = osc("type-settings /background\\\\r\\177\\177\\177\\177\\177\\177\\177102030\\\\r"),
        quit = osc("quit"),
        wait1 = dump_until(&dir, "\"seq\":1"),
        wait2 = dump_until(&dir, "focused[|]"),
        wait3 = dump_until(&dir, "#102030"),
        wait4 = dump_until(&dir, "edited[|]"),
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let all = dumps(&dir);
    assert!(all.len() >= 4, "{all:?}");
    for x in &all {
        check_dump(x);
        assert_eq!(x["window"], 1, "only the shell's tab dumps: {x}");
    }
    let with = |line: &str| {
        all.iter()
            .find(|x| screen(x).contains(&format!("\n{line}|")))
            .unwrap_or_else(|| panic!("no dump after {line}: {all:?}"))
    };
    // One Settings tab, in front after the second Cmd+, too.
    let after = with("focused");
    assert_eq!(num(after, "tabs"), 2.0, "{after}");
    assert_eq!(after["visible"], false, "Settings is in front: {after}");
    // The edit, written by the TUI into the file, recoloured this tab.
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("# mine") && text.contains("background = \"#102030\""), "{text}");
    let recoloured = all.iter().find(|x| x["background"] == "#102030").expect("never recoloured");
    assert!(!screen(recoloured).contains("terminal-decline:"), "{}", screen(recoloured));
    let last = with("edited");
    assert!(screen(last).starts_with("before|"), "{}", screen(last));
    assert!(
        !screen(last).contains("terminal-decline:"),
        "no notices while Settings is open: {}",
        screen(last)
    );
    let footprint = std::fs::read_to_string(dir.path().join("footprint.txt")).unwrap_or_default();
    let top = std::fs::read_to_string(dir.path().join("top.txt")).unwrap_or_default();
    eprintln!("prefs footprint:\n{footprint}\nprefs top:\n{top}");
    assert!(footprint.contains("Footprint") || footprint.contains("footprint"), "{footprint}");
}

/// Recording a shortcut in Settings: the TUI asks (OSC 7771), a Cmd key
/// posted through the window server is taken before the menus (Cmd+N
/// opens no window) and sent back, and the TUI binds it.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_settings_records_a_chord_before_the_menus() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("record");
    let file = dir.config_home().join("terminal-decline/config.toml");
    let (f, d) = (file.display(), dir.path().display());
    let wait_file =
        |what: &str| format!("i=0; until grep -q '{what}' '{f}' 2>/dev/null || [ $i -ge 100 ]; do sleep 0.1; i=$((i+1)); done; ");
    // Waits on the app itself, through this window's dumps, never on
    // time: until the preferences read keys (they set the tty raw with
    // TCSAFLUSH, which drops keys typed before), until they ask for a
    // chord, and until it was recorded.
    let dump_path = dir.dump_file();
    let wait_dump = |what: &str| {
        format!(
            "i=0; until {dump}sleep 0.1; tail -n 1 '{}' | grep -q '{what}' || [ $i -ge 150 ]; do i=$((i+1)); done; ",
            dump_path.display(),
            dump = osc("dump"),
        )
    };
    // In mkdir order: only the first window runs the test, so a window
    // Cmd+N opened by mistake shows up as a second dumper.
    let script = format!(
        "if mkdir '{d}/first' 2>/dev/null; then echo first; {settings}{ready}\
         {select}{recording}{key}{recorded}{yes}{wait}echo done; {dump}sleep 1; {quit}\
         else echo extra; {dump}fi; sleep 20",
        settings = osc("settings"),
        ready = wait_dump("\"settings_ready\":true"),
        recording = wait_dump("\"settings_recording\":true"),
        recorded = wait_dump("\"settings_recording\":false"),
        select = osc("type-settings \\\\e[C\\\\e[C\\\\e[B\\\\e[Bc"),
        key = osc("key 45 cmd"),
        yes = osc("type-settings y"),
        wait = wait_file("cmd+n"),
        dump = osc("dump"),
        quit = osc("quit"),
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    assert!(text.contains("\"cmd+n\" = \"new_tab\""), "{text}");
    assert!(text.contains("\"cmd+t\" = \"none\""), "{text}");
    let all = dumps(&dir);
    assert!(
        all.iter().all(|x| !screen(x).starts_with("extra|")),
        "Cmd+N reached the menus: {all:?}"
    );
    let last = all.last().expect("dumps");
    assert!(screen(last).contains("done|"), "{last}");
    assert_eq!(num(last, "tabs"), 2.0, "{last}");
}

/// Files dropped on the window are typed in as a paste: each path quoted
/// for the shell, separated by spaces, bracketed while the program asks
/// for bracketed paste. (The test-only `drop` command goes the same way as
/// a drag from Finder once the paths are off the pasteboard.)
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_dropped_files_are_typed_as_a_paste() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("drop");
    let d = dir.path().display();
    let first = ["/tmp/My Files/a (1).txt", "/tmp/$HOME & co"];
    let second = ["/Users/me/it is ~here"];
    let bracketed = format!("\x1b[200~{}\x1b[201~", terminal_decline_app::desktop::dropped_text(&first));
    let plain = terminal_decline_app::desktop::dropped_text(&second);
    let script = format!(
        "printf '\\033[?2004h'; stty -icanon -echo min 1 time 0; {drop1}\
         dd bs=1 count={n1} of='{d}/got1' 2>/dev/null; \
         printf '\\033[?2004l'; {drop2}\
         dd bs=1 count={n2} of='{d}/got2' 2>/dev/null; {quit}sleep 20",
        drop1 = osc(&format!("drop {}", first.join("\\t"))),
        drop2 = osc(&format!("drop {}", second.join("\\t"))),
        n1 = bracketed.len(),
        n2 = plain.len(),
        quit = osc("quit"),
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let got = |name: &str| std::fs::read_to_string(dir.path().join(name)).unwrap_or_default();
    assert_eq!(got("got1"), bracketed);
    assert_eq!(
        got("got1"),
        "\x1b[200~/tmp/My\\ Files/a\\ \\(1\\).txt /tmp/\\$HOME\\ \\&\\ co\x1b[201~"
    );
    assert_eq!(got("got2"), plain);
    assert_eq!(plain, "/Users/me/it\\ is\\ \\~here");
}

/// Cmd held over a link underlines it, OSC 8 or a plain URL, and stops
/// when Cmd is let go; Cmd-click opens http(s) links (stubbed under test
/// control, so they are listed in the dump) and refuses others with a
/// notice in the terminal.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_links_hover_and_open() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("links");
    let script = format!(
        "printf '\\033[?25lsee https://example.com/x now\\n\\033]8;;https://osc8.example/p\\007click me\\033]8;;\\007\\n'; \
         printf '\\033]8;;ssh://host\\007bad\\033]8;;\\007\\n'; {}{}{}{}{}{}{}{}{}{}{}{}sleep 20",
        osc("dump"),
        osc("hover 0 6"),
        osc("dump"),
        osc("hover 1 2"),
        osc("dump"),
        osc("hover off"),
        osc("dump"),
        osc("click 0 6"),
        osc("click 1 2"),
        osc("click 2 1"),
        osc("dump"),
        osc("quit"),
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 5, "{d:?}");
    let hovered: Vec<&Value> = d.iter().map(|x| &x["hovered_link"]).collect();
    assert_eq!(
        hovered,
        [
            &Value::Null,
            &"https://example.com/x".into(),
            &"https://osc8.example/p".into(),
            &Value::Null,
            &Value::Null
        ]
    );
    // One underline while a link is hovered.
    let rects = |x: &Value| num(x, "rects");
    assert_eq!(rects(&d[1]), rects(&d[0]) + 1.0, "{}", d[1]);
    assert_eq!(rects(&d[2]), rects(&d[0]) + 1.0, "{}", d[2]);
    assert_eq!(rects(&d[3]), rects(&d[0]));
    assert_eq!(
        d[4]["opened"],
        serde_json::json!(["https://example.com/x", "https://osc8.example/p"])
    );
    let s = screen(&d[4]);
    assert!(
        s.contains("terminal-decline: not opening ssh://host: only http, https, mailto and file links open"),
        "{s}"
    );
}

/// With prompts marked as the shell integration snippets mark them,
/// Cmd+Up and Cmd+Down scroll the view to the prompts in history, and
/// Select Command Output (its action sent as the menu sends it) selects
/// the output of the command at the top.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_jump_to_prompts_and_select_output() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("prompts");
    let wait = "i=0; while [ \"$(stty size)\" != '20 80' ] && [ $i -lt 100 ]; do sleep 0.05; i=$((i+1)); done; ";
    // Prompts at lines 0, 41 and 82 and the last at 123, each command
    // printing 40 lines.
    let session = "s() { printf '\\033]133;A\\007$ %s\\033]133;B\\007\\n\\033]133;C\\007' \"$1\"; \
                   i=0; while [ $i -lt 40 ]; do echo \"$1 $i\"; i=$((i+1)); done; printf '\\033]133;D;0\\007'; }; \
                   s one; s two; s three; printf '\\033]133;A\\007$ '; ";
    let up = osc("key 126 cmd");
    let down = osc("key 125 cmd");
    let dump = osc("dump");
    let script = format!(
        "{resize}{wait}{session}{dump}{up}{dump}{up}{dump}{select}{dump}{down}{dump}{down}{dump}{down}{dump}{quit}sleep 20",
        resize = osc("resize 80 20"),
        select = osc("menu select_output"),
        quit = osc("quit"),
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 7, "{d:?}");
    let offset = |x: &Value| num(x, "view_offset") as u64;
    let top = |x: &Value| screen(x).lines().next().unwrap_or_default().to_string();
    assert_eq!(offset(&d[0]), 0, "{}", d[0]);
    // The screen is lines 104..124: up to `three` (82), then `two` (41).
    assert_eq!((offset(&d[1]), top(&d[1])), (22, "$ three|".into()), "{}", d[1]);
    assert_eq!((offset(&d[2]), top(&d[2])), (63, "$ two|".into()), "{}", d[2]);
    let selected = d[3]["selection"].as_str().unwrap_or_default();
    let want: Vec<String> = (0..40).map(|i| format!("two {i}")).collect();
    assert_eq!(selected, want.join("\n"), "{}", d[3]);
    assert_eq!(offset(&d[4]), 22, "{}", d[4]);
    // The last prompt is on the screen: the view goes to the bottom, and
    // no further.
    assert_eq!(offset(&d[5]), 0, "{}", d[5]);
    assert_eq!(offset(&d[6]), 0, "{}", d[6]);
}

/// Option-click on the shell's command line (after the OSC 133 B mark)
/// types the arrow keys that move the shell's cursor to the cell, as the
/// program asked for them (application cursor mode: `ESC O`); on the
/// prompt, or with no marks, it types nothing.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_option_click_moves_the_shell_cursor() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("optclick");
    let wait = "i=0; while [ \"$(stty size)\" != '20 80' ] && [ $i -lt 100 ]; do sleep 0.05; i=$((i+1)); done; ";
    // The prompt, marked, with `echo hello` typed (the cursor after it, at
    // column 12) and the screen cleared before it.
    let line = "printf '\\033[H\\033[J\\033]133;A\\007$ \\033]133;B\\007echo hello'; ";
    // What a click typed is read with the terminal in raw mode.
    let script = format!(
        "{resize}{wait}stty raw -echo; \
         {line}{click_a}a=$(dd bs=1 count=24 2>/dev/null); \
         {line}printf '\\033[?1h\\033[8G'; {click_b}b=$(dd bs=1 count=9 2>/dev/null); \
         {line}{click_prompt}{click_far}stty min 0 time 10; c=$(dd bs=1 count=1 2>/dev/null); \
         printf '\\033[H\\033[J'; printf 'a=%s b=%s c=[%s]\\n' \"$a\" \"$b\" \"$c\" | tr -d '\\033'; \
         {dump}{quit}sleep 20",
        resize = osc("resize 80 20"),
        // The cursor is at column 12: 8 characters left to column 4.
        click_a = osc("click 0 4 option"),
        // Cursor at column 7, application cursor mode: 3 right.
        click_b = osc("click 0 10 option"),
        // The prompt (column 1) and another row type nothing.
        click_prompt = osc("click 0 1 option"),
        click_far = osc("click 5 3 option"),
        dump = osc("dump"),
        quit = osc("quit"),
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 1, "{d:?}");
    let s = screen(&d[0]);
    let want = format!("a={} b={} c=[]", "[D".repeat(8), "OC".repeat(3));
    assert!(s.starts_with(&want), "{s}");
}

/// Alex's two selections: over a program that takes the mouse, a
/// Shift-drag makes a terminal selection (the program gets nothing); a
/// plain click goes to the program and clears it; a Shift-click without
/// a drag selects nothing; and a key typed clears a selection, so Cmd+C
/// never copies an old one.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_shift_drag_selects_over_a_program_taking_the_mouse() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("shiftdrag");
    let wait = "i=0; while [ \"$(stty size)\" != '20 80' ] && [ $i -lt 100 ]; do sleep 0.05; i=$((i+1)); done; ";
    let script = format!(
        "{resize}{wait}stty raw -echo; \
         printf '\\033[H\\033[Jhello world\\033[?1002h\\033[?1006h'; \
         {shift_down}{shift_drag}{shift_up}{dump}\
         {down}{up}r=$(dd bs=1 count=18 2>/dev/null); {dump}\
         {click_down}{click_up}{dump}\
         {word_down}{word_drag}{word_up}{dump}\
         {key}k=$(dd bs=1 count=1 2>/dev/null); {dump}\
         printf '\\033[?1002l\\033[?1006l\\033[H\\033[J'; printf 'r=%s k=%s\\n' \"$r\" \"$k\" | tr -d '\\033'; \
         {dump}{quit}sleep 20",
        resize = osc("resize 80 20"),
        shift_down = osc("mouse down 0 0 shift"),
        shift_drag = osc("mouse drag 0 4 shift"),
        shift_up = osc("mouse up 0 4 shift"),
        // A plain click goes to the program: SGR reports at column 7.
        down = osc("mouse down 0 6"),
        up = osc("mouse up 0 6"),
        click_down = osc("mouse down 0 2 shift"),
        click_up = osc("mouse up 0 2 shift"),
        word_down = osc("mouse down 0 6 shift"),
        word_drag = osc("mouse drag 0 10 shift"),
        word_up = osc("mouse up 0 10 shift"),
        key = osc("key 0"),
        dump = osc("dump"),
        quit = osc("quit"),
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 6, "{d:?}");
    let selection = |x: &Value| x["selection"].as_str().map(str::to_owned);
    assert_eq!(selection(&d[0]).as_deref(), Some("hello"), "{}", d[0]);
    assert_eq!(selection(&d[1]), None, "a click sent to the program clears it: {}", d[1]);
    assert_eq!(selection(&d[2]), None, "a Shift-click alone selects nothing: {}", d[2]);
    assert_eq!(selection(&d[3]).as_deref(), Some("world"), "{}", d[3]);
    assert_eq!(selection(&d[4]), None, "a key typed clears it: {}", d[4]);
    let s = screen(&d[5]);
    assert!(
        s.starts_with("r=[<0;7;1M[<0;7;1m k=a"),
        "only the plain click went to the program: {s}"
    );
}

/// A folder opened with the app (`open -a`, as a folder dropped on its
/// Dock icon is): an app that isn't running starts with a window there
/// and no other, and a running one opens another window (or a tab, as the
/// macOS tab setting says) in the next folder.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_folders_opened_with_the_app() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("open-folder");
    let sub = |name: &str| {
        let p = dir.path().join(name);
        std::fs::create_dir_all(&p).unwrap();
        p.display().to_string()
    };
    let (a, b) = (sub("dir-a"), sub("dir b"));
    let name = format!("Terminal Decline Open {}", std::process::id());
    let app_path = dir.path().join("Applications").join(format!("{name}.app"));
    let d = dir.path().display();
    let two_dumped = format!("[ \"$(wc -l < '{}')\" -ge 2 ]", dir.dump_file().display());
    let script = format!(
        "echo \"in $(basename \"$PWD\")\"; {dump}\
         if mkdir '{d}/first' 2>/dev/null; then \
           /usr/bin/open -a '{app}' '{b}'; \
           i=0; while ! {two_dumped} && [ $i -lt 200 ]; do sleep 0.05; i=$((i+1)); done; {quit}\
         fi; sleep 20",
        dump = osc("dump"),
        quit = osc("quit"),
        app = app_path.display(),
    );
    let app = temp_bundle(&dir, &name, &script);
    assert_eq!(app, app_path);
    let mut open = Command::new("/usr/bin/open");
    open.args(["-g", "-n", "-W", "--env"])
        .arg(format!("DECLINE_TEST_DUMP={}", dir.dump_file().display()))
        .arg("--env")
        .arg(format!("XDG_CONFIG_HOME={}", dir.config_home().display()))
        .arg("--stderr")
        .arg(dir.path().join("stderr.txt"))
        .arg("-a")
        .arg(&app)
        .arg(&a);
    let (status, _) = run(&mut open);
    let lsregister = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    let _ = Command::new(lsregister).arg("-u").arg(&app).status();
    let stderr = std::fs::read_to_string(dir.path().join("stderr.txt")).unwrap_or_default();
    assert!(status.success(), "open: {status}; the app's stderr:\n{stderr}");
    let all = dumps(&dir);
    assert_eq!(all.len(), 2, "{all:?}\nstderr:\n{stderr}");
    let place = |x: &Value| (num(x, "window") as u32, screen(x).lines().next().unwrap_or("").to_string());
    assert_eq!(place(&all[0]), (1, "in dir-a|".into()), "{all:?}");
    assert_eq!(place(&all[1]), (2, "in dir b|".into()), "{all:?}");
}

/// `confirm_close = true`: closing a window while a program runs asks
/// first (a sheet, and the window stays); at the shell's prompt it closes
/// at once.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_confirm_close_asks_while_a_program_runs() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("confirm");
    std::fs::create_dir_all(dir.config_home().join("terminal-decline")).unwrap();
    std::fs::write(dir.config_home().join("terminal-decline/config.toml"), "confirm_close = true\n").unwrap();
    // A job in the foreground, as an interactive shell runs one.
    let job = dir.path().join("job.sh");
    std::fs::write(
        &job,
        format!(
            "{}{}sleep 1; {}{}sleep 20\n",
            in_foreground(),
            osc("close"),
            osc("dump"),
            osc("quit")
        ),
    )
    .unwrap();
    let (status, _) = decline(&dir, &format!("set -m; /bin/sh '{}'; sleep 20", job.display()), true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0]["sheet"], true, "{}", d[0]);

    // Nothing running: the window closes, and with it the app.
    let dir = TempDir::new("confirm-idle");
    std::fs::create_dir_all(dir.config_home().join("terminal-decline")).unwrap();
    std::fs::write(dir.config_home().join("terminal-decline/config.toml"), "confirm_close = true\n").unwrap();
    let (status, elapsed) = decline(&dir, &format!("{}sleep 20", osc("close")), true);
    assert!(status.success(), "{status}");
    assert!(elapsed < Duration::from_secs(15), "closed after {elapsed:?}");
    assert!(dumps(&dir).is_empty());
}

/// A shell loop that waits (at most 10 s) until `cond` holds.
fn wait_for(cond: &str) -> String {
    format!("i=0; while ! {{ {cond}; }} && [ $i -lt 200 ]; do sleep 0.05; i=$((i+1)); done; ")
}

/// A shell snippet that asks for a dump and waits (at most 10 s) until
/// it is written.
fn dump_and_wait(dir: &TempDir) -> String {
    let file = dir.dump_file();
    let count = format!("$(cat '{}' 2>/dev/null | wc -l)", file.display());
    format!("n={count}; {}{}", osc("dump"), wait_for(&format!("[ {count} -gt $n ]")))
}

/// Splits run the user's login shell: these tests make it zsh with its
/// startup files from `dir` (a `.zshrc` that does what the test needs).
fn with_zsh(cmd: &mut Command, dir: &TempDir, zshrc: &str) {
    std::fs::write(dir.path().join(".zshrc"), zshrc).unwrap();
    cmd.env("SHELL", "/bin/zsh").env("ZDOTDIR", dir.path());
}

fn panes(d: &Value) -> &Vec<Value> {
    d["panes"].as_array().unwrap_or_else(|| panic!("no panes in {d}"))
}

fn pane_screen(p: &Value) -> &str {
    p["screen"].as_str().unwrap()
}

/// Cmd+D splits the focused pane: the new pane runs the user's shell in
/// the pane's folder, gets focus and the keys, and both programs are told
/// their sizes. Focus moves with the `focus` control; Cmd+W closes the
/// focused pane and leaves the other running at its old size; a pane whose
/// program exits closes.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_split_focus_and_close_panes() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("split");
    let d = dir.path().display();
    // The panes the splits open run this; `role` says what to do.
    let zshrc = format!(
        r#"PS1='%% '
case "$(cat '{d}/role' 2>/dev/null)" in
  echo) echo "two in ${{PWD:t}}"; stty size > '{d}/size2'; touch '{d}/two-ready'
        read -r line; print -r -- "got:$line" > '{d}/two-got'; sleep 20 ;;
  exit) touch '{d}/three-ran'; exit 0 ;;
esac
"#
    );
    let script = format!(
        "cd '{d}'; s0=$(stty size); echo \"one $s0\"; echo echo > '{d}/role'; {split}\
         {wait_two}{wait_size}echo \"one now $(stty size)\"; {dump}\
         {keys}{wait_got}{dump}\
         {focus0}{dump}\
         {focus1}{cmd_w}{wait_back}{dump}\
         echo exit > '{d}/role'; {split_down}{wait_three}\
         j=0; while [ $j -lt 50 ]; do {dump}tail -1 '{file}' | grep -q '\"dividers\":0' && break; sleep 0.1; j=$((j+1)); done; \
         {quit}sleep 20",
        split = osc("split right"),
        split_down = osc("split down"),
        wait_two = wait_for(&format!("[ -e '{d}/two-ready' ]")),
        wait_size = wait_for("[ \"$(stty size)\" != \"$s0\" ]"),
        wait_got = wait_for(&format!("[ -s '{d}/two-got' ]")),
        wait_back = wait_for("[ \"$(stty size)\" = \"$s0\" ]"),
        wait_three = wait_for(&format!("[ -e '{d}/three-ran' ]")),
        dump = dump_and_wait(&dir),
        // q z q Return, typed as keys into the key window.
        keys = [osc("key 12"), osc("key 6"), osc("key 12"), osc("key 36")].concat(),
        focus0 = osc("focus 0"),
        focus1 = osc("focus 1"),
        cmd_w = osc("key 13 cmd"),
        file = dir.dump_file().display(),
        quit = osc("quit"),
    );
    let mut cmd = Command::new(APP);
    cmd.args(["-e", "/bin/sh", "-c", &script])
        .env("XDG_CONFIG_HOME", dir.config_home())
        .env("DECLINE_TEST_DUMP", dir.dump_file())
        .current_dir(dir.path());
    with_zsh(&mut cmd, &dir, &zshrc);
    let (status, _) = run(&mut cmd);
    assert!(status.success(), "{status}");
    let all = dumps(&dir);
    assert!(all.len() >= 5, "{all:?}");
    let (split, typed, back, closed, last) = (&all[0], &all[1], &all[2], &all[3], all.last().unwrap());
    // The dumps taken while the third pane is still open show it focused
    // and blank (no glyphs), so only these get the usual checks.
    for x in [split, typed, back, closed, last] {
        check_dump(x);
    }
    let first = &panes(split)[0];
    let first_line = pane_screen(first).lines().next().unwrap();
    let full: Vec<u32> = first_line
        .trim_start_matches("one ")
        .trim_end_matches('|')
        .split(' ')
        .map(|n| n.parse().unwrap())
        .collect();

    // Split right: two panes side by side, the new one focused, one divider.
    let p = panes(split);
    assert_eq!((p.len(), num(split, "dividers")), (2, 1.0), "{split}");
    assert_eq!(
        (p[0]["focused"].clone(), p[1]["focused"].clone()),
        (false.into(), true.into()),
        "{split}"
    );
    let rect = |p: &Value| -> Vec<i64> { p["rect"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect() };
    let (a, b) = (rect(&p[0]), rect(&p[1]));
    assert_eq!((a[0], a[1], b[1]), (0, 0, 0), "{split}");
    assert!(b[0] > a[0] + a[2] - 1 && b[3] == a[3], "{a:?} {b:?}");
    let base = dir.path().file_name().unwrap().to_string_lossy().into_owned();
    assert!(pane_screen(&p[1]).starts_with(&format!("two in {base}|")), "{}", pane_screen(&p[1]));
    // Each program was told its pane's size; the first pane got narrower.
    let grid = |p: &Value| format!("{} {}", p["lines"], p["cols"]);
    assert!(
        pane_screen(&p[0]).contains(&format!("one now {}|", grid(&p[0]))),
        "{}",
        pane_screen(&p[0])
    );
    assert_eq!(std::fs::read_to_string(dir.path().join("size2")).unwrap().trim(), grid(&p[1]));
    assert!((p[0]["cols"].as_u64().unwrap() as u32) < full[1], "{split}");
    assert_eq!(p[0]["lines"].as_u64().unwrap() as u32, full[0], "{split}");

    // The keys went to the focused (new) pane only.
    let p = panes(typed);
    assert_eq!(std::fs::read_to_string(dir.path().join("two-got")).unwrap().trim(), "got:qzq");
    assert!(pane_screen(&p[1]).contains("qzq"), "{}", pane_screen(&p[1]));
    assert!(!pane_screen(&p[0]).contains("qzq"), "{}", pane_screen(&p[0]));

    // `focus 0` moved focus back.
    let p = panes(back);
    assert_eq!(
        (p[0]["focused"].clone(), p[1]["focused"].clone()),
        (true.into(), false.into()),
        "{back}"
    );

    // Cmd+W in the second pane closed it; the first, still running, fills
    // the window again.
    let p = panes(closed);
    assert_eq!((p.len(), num(closed, "dividers")), (1, 0.0), "{closed}");
    assert_eq!(p[0]["id"], panes(split)[0]["id"]);
    assert_eq!(
        (num(closed, "lines") as u32, num(closed, "cols") as u32),
        (full[0], full[1]),
        "{closed}"
    );

    // The third pane's shell exited at once: its pane closed.
    assert!(dir.path().join("three-ran").exists());
    assert_eq!((panes(last).len(), num(last, "dividers")), (1, 0.0), "{last}");
}

/// With `confirm_close`, closing a pane whose program is running asks
/// first; a pane at its shell's prompt closes without asking.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_confirm_close_asks_for_a_pane() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("confirm-pane");
    let d = dir.path().display();
    std::fs::create_dir_all(dir.config_home().join("terminal-decline")).unwrap();
    std::fs::write(dir.config_home().join("terminal-decline/config.toml"), "confirm_close = true\n").unwrap();
    let zshrc = format!(
        r#"PS1='%% '
if [ "$(cat '{d}/role' 2>/dev/null)" = job ]; then touch '{d}/job'; sleep 30; fi
touch '{d}/prompt'
"#
    );
    let script = format!(
        "echo job > '{d}/role'; {split}{wait_job}sleep 1; {close}{dump}\
         {quit}sleep 20",
        split = osc("split right"),
        wait_job = wait_for(&format!("[ -e '{d}/job' ]")),
        close = osc("close-pane"),
        dump = dump_and_wait(&dir),
        quit = osc("quit"),
    );
    let mut cmd = Command::new(APP);
    cmd.args(["-e", "/bin/sh", "-c", &script])
        .env("XDG_CONFIG_HOME", dir.config_home())
        .env("DECLINE_TEST_DUMP", dir.dump_file())
        .current_dir(dir.path());
    with_zsh(&mut cmd, &dir, &zshrc);
    let (status, _) = run(&mut cmd);
    assert!(status.success(), "{status}");
    let all = dumps(&dir);
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(all[0]["sheet"], true, "{}", all[0]);
    assert_eq!(panes(&all[0]).len(), 2, "{}", all[0]);

    // At the prompt: closed at once.
    let dir = TempDir::new("confirm-pane-idle");
    let d = dir.path().display();
    std::fs::create_dir_all(dir.config_home().join("terminal-decline")).unwrap();
    std::fs::write(dir.config_home().join("terminal-decline/config.toml"), "confirm_close = true\n").unwrap();
    let zshrc = format!("PS1='%% '\ntouch '{d}/prompt'\n");
    let script = format!(
        "s0=$(stty size); {split}{wait_prompt}sleep 0.5; {close}{wait_back}{dump}{quit}sleep 20",
        split = osc("split right"),
        wait_prompt = wait_for(&format!("[ -e '{d}/prompt' ]")),
        close = osc("close-pane"),
        wait_back = wait_for("[ \"$(stty size)\" = \"$s0\" ]"),
        dump = dump_and_wait(&dir),
        quit = osc("quit"),
    );
    let mut cmd = Command::new(APP);
    cmd.args(["-e", "/bin/sh", "-c", &script])
        .env("XDG_CONFIG_HOME", dir.config_home())
        .env("DECLINE_TEST_DUMP", dir.dump_file())
        .current_dir(dir.path());
    with_zsh(&mut cmd, &dir, &zshrc);
    let (status, _) = run(&mut cmd);
    assert!(status.success(), "{status}");
    let all = dumps(&dir);
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(all[0]["sheet"], false, "{}", all[0]);
    assert_eq!(panes(&all[0]).len(), 1, "{}", all[0]);
}

/// A window saved with three panes reopens with them after a restart:
/// the same shares, each pane in its folder, the window's program in the
/// panes that ran it and the user's shell in the split, with the command
/// it was running typed at its prompt, and the same pane focused. Saved
/// again, it gives back what it was restored from.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_restore_round_trips_panes() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("restore-panes");
    let sub = |name: &str| {
        let p = dir.path().join(name);
        std::fs::create_dir_all(&p).unwrap();
        p.display().to_string()
    };
    let (a, b, c) = (sub("pane-a"), sub("pane-b"), sub("pane-c"));
    let layout = serde_json::json!({
        "root": {"split": {"dir": "right", "ratio": 0.4,
            "a": {"pane": {"cwd": a, "program": true}},
            "b": {"split": {"dir": "down", "ratio": 0.5,
                "a": {"pane": {"cwd": b, "program": true}},
                "b": {"pane": {"cwd": c, "command": "touch 'ran here'"}}}}}},
        "focused": 1
    });
    let restore = dir.path().join("restore.json");
    let saved = serde_json::json!([{"cwd": b, "frame": [100.0, 100.0, 900.0, 600.0], "panes": layout}]);
    std::fs::write(&restore, saved.to_string()).unwrap();
    let file = dir.dump_file().display().to_string();
    // The split's shell dumps once the command is in its line editor.
    let zshrc = format!(
        r#"PS1='READY> '
td_seen=
td-line-pre-redraw() {{
  [[ -n $BUFFER && -z $td_seen ]] || return 0
  td_seen=1
  {{ sleep 0.3; {dump}}} > /dev/tty 2>&1 &!
}}
zle -N zle-line-pre-redraw td-line-pre-redraw
"#,
        dump = osc("dump"),
    );
    // The panes running the window's program each dump; the first there
    // waits for the shell's dump, saves and quits.
    let saved_file = format!("{file}.saved");
    let script = format!(
        "echo \"in $(basename \"$PWD\")\"; {dump}if mkdir '{d}/saver' 2>/dev/null; then \
           {wait_typed}{dump_last}{save}{wait_saved}{quit}fi; sleep 20",
        d = dir.path().display(),
        dump = osc("dump"),
        wait_typed = wait_for(&format!("grep -q 'READY> touch' '{file}' 2>/dev/null")),
        dump_last = dump_and_wait(&dir),
        save = osc("save"),
        wait_saved = wait_for(&format!("[ -s '{saved_file}' ]")),
        quit = osc("quit"),
    );
    let mut cmd = Command::new(APP);
    cmd.arg("--restore")
        .arg(&restore)
        .args(["-e", "/bin/sh", "-c", &script])
        .env("XDG_CONFIG_HOME", dir.config_home())
        .env("DECLINE_TEST_DUMP", dir.dump_file())
        .current_dir(dir.path());
    with_zsh(&mut cmd, &dir, &zshrc);
    let (status, _) = run(&mut cmd);
    assert!(status.success(), "{status}");
    let all = dumps(&dir);
    let last = all.last().unwrap_or_else(|| panic!("no dumps"));
    check_dump(last);
    let p = panes(last);
    assert_eq!((p.len(), num(last, "dividers")), (3, 2.0), "{last}");
    assert!(pane_screen(&p[0]).starts_with("in pane-a|"), "{}", pane_screen(&p[0]));
    assert!(pane_screen(&p[1]).starts_with("in pane-b|"), "{}", pane_screen(&p[1]));
    assert!(pane_screen(&p[2]).contains("READY> touch 'ran here'|"), "{}", pane_screen(&p[2]));
    assert!(!dir.path().join("pane-c/ran here").exists(), "it ran without Enter");
    let focused: Vec<bool> = p.iter().map(|p| p["focused"] == true).collect();
    assert_eq!(focused, [false, true, false], "{last}");
    // Left 40%, the right half stacked.
    let rect = |p: &Value| -> Vec<i64> { p["rect"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect() };
    let (ra, rb, rc) = (rect(&p[0]), rect(&p[1]), rect(&p[2]));
    let width = rb[0] + rb[2];
    let share = ra[2] as f64 / width as f64;
    assert!((share - 0.4).abs() < 0.02, "{ra:?} {rb:?}");
    assert_eq!((rb[0], rc[0]), (rc[0], rb[0]));
    assert!(rb[1] == 0 && rc[1] > rb[1] + rb[3] - 1, "{rb:?} {rc:?}");

    // Saved again: the same layout.
    let again: Value = serde_json::from_str(&std::fs::read_to_string(&saved_file).unwrap()).unwrap();
    let again = &again[0]["panes"];
    assert_eq!(again["focused"], 1, "{again}");
    let root = &again["root"]["split"];
    assert_eq!(root["dir"], "right");
    assert!((root["ratio"].as_f64().unwrap() - 0.4).abs() < 1e-6, "{root}");
    assert_eq!(root["a"]["pane"]["cwd"], a.as_str());
    assert_eq!(root["a"]["pane"]["program"], true);
    let right = &root["b"]["split"];
    assert_eq!(right["dir"], "down");
    assert_eq!(right["a"]["pane"]["cwd"], b.as_str());
    assert_eq!(right["a"]["pane"]["program"], true);
    assert_eq!(right["b"]["pane"]["cwd"], c.as_str());
    assert!(right["b"]["pane"].get("program").is_none(), "{right}");
}

/// A window saved with a pane zoomed reopens zoomed: that pane fills the
/// window, the other runs hidden, and saved again it is still zoomed.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_restore_keeps_a_zoom() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("restore-zoom");
    let sub = |name: &str| {
        let p = dir.path().join(name);
        std::fs::create_dir_all(&p).unwrap();
        p.display().to_string()
    };
    let (a, b) = (sub("pane-a"), sub("pane-b"));
    let layout = serde_json::json!({
        "root": {"split": {"dir": "right", "ratio": 0.5,
            "a": {"pane": {"cwd": a, "program": true}},
            "b": {"pane": {"cwd": b, "program": true}}}},
        "focused": 1,
        "zoomed": 1
    });
    let restore = dir.path().join("restore.json");
    let saved = serde_json::json!([{"cwd": b, "frame": [100.0, 100.0, 900.0, 600.0], "panes": layout}]);
    std::fs::write(&restore, saved.to_string()).unwrap();
    let saved_file = format!("{}.saved", dir.dump_file().display());
    // Both panes run this; the zoomed one dumps, saves and quits.
    let script = format!(
        "echo \"in $(basename \"$PWD\")\"; if [ \"$(basename \"$PWD\")\" = pane-b ]; then \
           {dump}{save}{wait_saved}{quit}fi; sleep 20",
        dump = dump_and_wait(&dir),
        save = osc("save"),
        wait_saved = wait_for(&format!("[ -s '{saved_file}' ]")),
        quit = osc("quit"),
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
    let all = dumps(&dir);
    let last = all.last().unwrap_or_else(|| panic!("no dumps"));
    check_dump(last);
    let p = panes(last);
    assert_eq!((p.len(), num(last, "dividers")), (2, 0.0), "{last}");
    assert_eq!(last["zoomed"], p[1]["id"], "{last}");
    assert_eq!(
        (p[0]["shown"].clone(), p[1]["shown"].clone()),
        (false.into(), true.into()),
        "{last}"
    );
    assert_eq!(p[1]["focused"], true, "{last}");
    assert!(pane_screen(&p[1]).starts_with("in pane-b|"), "{}", pane_screen(&p[1]));
    // The zoomed pane has the whole window; the hidden one the half it
    // gets back.
    assert_eq!(
        (num(last, "cols"), num(last, "lines")),
        (p[1]["cols"].as_f64().unwrap(), p[1]["lines"].as_f64().unwrap())
    );
    assert!(p[0]["cols"].as_u64().unwrap() < p[1]["cols"].as_u64().unwrap() / 2 + 2, "{last}");

    let again: Value = serde_json::from_str(&std::fs::read_to_string(&saved_file).unwrap()).unwrap();
    let again = &again[0]["panes"];
    assert_eq!((again["focused"].clone(), again["zoomed"].clone()), (1.into(), 1.into()), "{again}");
}

/// A shell loop that dumps (at most 50 times) until a dump has `text`.
fn until_dumped(dir: &TempDir, text: &str) -> String {
    format!(
        "k=0; while [ $k -lt 50 ]; do {dump}grep -qF '{text}' '{file}' && break; sleep 0.1; k=$((k+1)); done; ",
        dump = dump_and_wait(dir),
        file = dir.dump_file().display(),
    )
}

/// A panic while building one pane's frame stays in that pane: its
/// program is stopped and it shows where the crash log is, while the
/// other pane keeps drawing. A second one soon after stops that pane
/// being built at all.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_a_crash_while_drawing_stays_in_its_pane() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("build-panic");
    let d = dir.path().display();
    let zshrc = format!("PS1='%% '\ntouch '{d}/two-ready'\nsleep 30\n");
    let script = format!(
        "{split}{wait_two}{panic_locked}{wait_crash}{dump}echo still here; {dump}\
         {panic}{wait_twice}{dump}echo and again; {dump}{quit}sleep 20",
        split = osc("split right"),
        wait_two = wait_for(&format!("[ -e '{d}/two-ready' ]")),
        // First while its snapshot is taken (terminal locked), then while
        // it is built (unlocked).
        panic_locked = osc("build-panic 1 snapshot"),
        panic = osc("build-panic 1"),
        // Dumps until the split's crash shows in one (at most 50).
        wait_crash = until_dumped(&dir, "\"crashes\":1"),
        wait_twice = until_dumped(&dir, "\"crashes\":2"),
        dump = dump_and_wait(&dir),
        quit = osc("quit"),
    );
    let mut cmd = Command::new(APP);
    cmd.args(["-e", "/bin/sh", "-c", &script])
        .env("XDG_CONFIG_HOME", dir.config_home())
        .env("DECLINE_TEST_DUMP", dir.dump_file())
        // The crash logs go under the temp dir, not the user's.
        .env("HOME", dir.path())
        .current_dir(dir.path());
    with_zsh(&mut cmd, &dir, &zshrc);
    let (status, _) = run(&mut cmd);
    assert!(status.success(), "{status}");
    let all = dumps(&dir);
    let crashes = |d: &Value| panes(d)[1]["crashes"].as_u64().unwrap();
    let builds = |d: &Value, i: usize| panes(d)[i]["builds"].as_u64().unwrap();
    let once = all
        .iter()
        .position(|d| crashes(d) == 1)
        .unwrap_or_else(|| panic!("no crash: {all:?}"));
    let twice = all
        .iter()
        .position(|d| crashes(d) == 2)
        .unwrap_or_else(|| panic!("no second crash: {all:?}"));
    // The dumps after each: `dump`, `echo`, `dump`.
    let (first, still, second, again) = (&all[once + 1], &all[once + 2], &all[twice + 1], &all[twice + 2]);
    for d in [first, still, second, again] {
        assert_eq!(panes(d).len(), 2, "{d}");
    }
    // Long rows wrap, marked with ⏎.
    let split = pane_screen(&panes(first)[1]).replace("⏎\n", "");
    assert!(
        split.contains("hit a bug and stopped this session; details in ~/Library/Logs/terminal-decline/crash-"),
        "{split}"
    );
    let logs = dir.path().join("Library/Logs/terminal-decline");
    assert!(std::fs::read_dir(&logs).unwrap().count() >= 2, "a log per crash");
    // The other pane kept drawing.
    assert!(pane_screen(&panes(still)[0]).contains("still here|"), "{still}");
    assert!(builds(still, 0) > builds(first, 0), "{first} {still}");
    assert!(pane_screen(&panes(again)[0]).contains("and again|"), "{again}");
    assert!(builds(again, 0) > builds(second, 0), "{second} {again}");
    // After the second crash the split isn't built any more.
    assert_eq!(builds(again, 1), builds(second, 1), "{second} {again}");
}

/// The last `two L C` line pane 2's program printed: the size it was
/// last told.
fn last_told(p: &Value) -> String {
    let screen = pane_screen(p);
    let row = screen.lines().map(|r| r.trim_end_matches('|')).rfind(|r| r.starts_with("two "));
    row.unwrap_or_else(|| panic!("no size in {screen}"))
        .trim_start_matches("two ")
        .trim()
        .to_string()
}

/// Dividers drag, move a cell with Cmd+Ctrl+arrows and even out, and the
/// focused pane zooms with Cmd+Shift+Enter: each pane's program is told
/// its new size as its grid changes; a zoomed pane fills the window while
/// the other is hidden but keeps running at its size; zooming again, or
/// focusing the hidden pane, brings them all back.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_drag_resize_equalize_and_zoom_panes() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("drag-zoom");
    let d = dir.path().display();
    // The split prints its size whenever it changes, and keeps it in a file.
    let zshrc = format!(
        r#"PS1='%% '
last=
while :; do s=$(stty size); if [ "$s" != "$last" ]; then echo "two $s"; print -r -- "$s" > '{d}/size1.new'; mv '{d}/size1.new' '{d}/size1'; last=$s; fi; sleep 0.05; done
"#
    );
    // `step ACTION`: do it, wait until the split's size changes, then let
    // this pane print its own and dump.
    let step = |action: &str| {
        format!(
            "p=$(cat '{d}/size1'); {action}{wait}echo \"one $(stty size)\"; {dump}",
            wait = wait_for(&format!("[ \"$(cat '{d}/size1')\" != \"$p\" ]")),
            dump = dump_and_wait(&dir),
        )
    };
    let script = format!(
        "full=$(stty size); echo \"full $full\"; {split}{wait_split}{drag}{right}{equalize}{zoom}{unzoom}{zoom_again}{focus0}{quit}sleep 20",
        split = osc("split right"),
        wait_split = wait_for(&format!("[ -s '{d}/size1' ]")),
        drag = step(&osc("drag 0 300")),
        // Cmd+Ctrl+→ in the split (focused): its divider one cell right.
        right = step(&osc("key 124 cmd+ctrl")),
        equalize = step(&osc("key 24 cmd+ctrl")),
        zoom = step(&osc("key 36 cmd+shift")),
        unzoom = step(&osc("zoom")),
        zoom_again = step(&osc("zoom")),
        focus0 = step(&osc("focus 0")),
        quit = osc("quit"),
    );
    let mut cmd = Command::new(APP);
    cmd.args(["-e", "/bin/sh", "-c", &script])
        .env("XDG_CONFIG_HOME", dir.config_home())
        .env("DECLINE_TEST_DUMP", dir.dump_file())
        .current_dir(dir.path());
    with_zsh(&mut cmd, &dir, &zshrc);
    let (status, _) = run(&mut cmd);
    assert!(status.success(), "{status}");
    let all = dumps(&dir);
    assert_eq!(all.len(), 7, "{all:?}");
    for x in &all {
        check_dump(x);
    }
    let rect = |p: &Value| -> Vec<i64> { p["rect"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect() };
    let grid = |p: &Value| format!("{} {}", p["lines"], p["cols"]);
    // Each shown pane's program was told its grid.
    let told = |d: &Value| {
        let p = panes(d);
        let one = pane_screen(&p[0])
            .lines()
            .map(|r| r.trim_end_matches('|'))
            .rfind(|r| r.starts_with("one "))
            .unwrap_or("")
            .to_string();
        if p[0]["shown"] == true {
            assert_eq!(one.trim(), format!("one {}", grid(&p[0])), "{d}");
        }
        if p[1]["shown"] == true {
            assert_eq!(last_told(&p[1]), grid(&p[1]), "{d}");
        }
    };
    let (dragged, right, even, zoomed, back, again, focused) = (&all[0], &all[1], &all[2], &all[3], &all[4], &all[5], &all[6]);
    let full = pane_screen(&panes(dragged)[0])
        .split('|')
        .next()
        .unwrap()
        .trim_start_matches("full ")
        .trim()
        .to_string();
    let cell = num(dragged, "cell_width") as i64;

    // Dragged: the first pane is 300 px wide, the second takes the rest.
    let p = panes(dragged);
    assert_eq!(rect(&p[0])[2], 300, "{dragged}");
    assert!(rect(&p[1])[0] > 300, "{dragged}");
    told(dragged);

    // Cmd+Ctrl+→: one cell wider.
    let p = panes(right);
    assert_eq!(rect(&p[0])[2], 300 + cell, "{right}");
    told(right);

    // Cmd+Ctrl+=: even shares.
    let p = panes(even);
    assert!((rect(&p[0])[2] - rect(&p[1])[2]).abs() <= 1, "{even}");
    told(even);
    let even_grid = grid(&p[1]);

    // Cmd+Shift+Enter: the split fills the window; the first pane is
    // hidden and keeps the size it had.
    let p = panes(zoomed);
    assert_eq!(
        (zoomed["zoomed"].clone(), num(zoomed, "dividers")),
        (p[1]["id"].clone(), 0.0),
        "{zoomed}"
    );
    assert_eq!(
        (p[0]["shown"].clone(), p[1]["shown"].clone()),
        (false.into(), true.into()),
        "{zoomed}"
    );
    assert_eq!((rect(&p[1])[0], rect(&p[1])[1]), (0, 0), "{zoomed}");
    assert_eq!(grid(&p[1]), full, "{zoomed}");
    assert_eq!(grid(&p[0]), grid(&panes(even)[0]), "{zoomed}");
    told(zoomed);

    // Zoomed again: back as they were. Then zoomed, and focusing the
    // hidden pane ended the zoom.
    for d in [back, focused] {
        let p = panes(d);
        assert_eq!((d["zoomed"].clone(), num(d, "dividers")), (Value::Null, 1.0), "{d}");
        assert!(p.iter().all(|p| p["shown"] == true), "{d}");
        assert_eq!(grid(&p[1]), even_grid, "{d}");
        told(d);
    }
    assert_eq!(again["zoomed"], panes(again)[1]["id"], "{again}");
    assert_eq!(panes(focused)[0]["focused"], true, "{focused}");
}

/// The find bar (E.7): Cmd+F opens it with the keys in its field, typing
/// searches after the field's delay, the count says which hit is current,
/// Find Next and Find Previous move the view to their hit (wrapping),
/// and Esc closes it.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_find_bar_searches_history() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("find");
    let wait = "i=0; while [ \"$(stty size)\" != '20 80' ] && [ $i -lt 100 ]; do sleep 0.05; i=$((i+1)); done; ";
    // Lines 0..60, "needle" on 5, 25 and 45; the screen shows 41..60.
    let lines =
        "i=0; while [ $i -lt 60 ]; do if [ $((i % 20)) -eq 5 ]; then echo \"row $i needle\"; else echo \"row $i\"; fi; i=$((i+1)); done; ";
    // n, e, e typed into the field.
    let typed = [osc("key 45"), osc("key 14"), osc("key 14")].concat();
    let script = format!(
        "{resize}{wait}{lines}{find}{dump}{typed}sleep 1; {dump}{next}{dump}{previous}{dump}{enter}{dump}{set}{dump}{esc}{dump}{quit}sleep 20",
        enter = osc("key 36"),
        resize = osc("resize 80 20"),
        find = osc("menu find"),
        dump = dump_and_wait(&dir),
        next = osc("menu find_next"),
        previous = osc("menu find_previous"),
        set = osc("find row 2"),
        esc = osc("key 53"),
        quit = osc("quit"),
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 7, "{d:?}");
    let find = |x: &Value| x["find"].clone();
    let offset = |x: &Value| num(x, "view_offset") as u64;
    // Open, empty, the field has the keys.
    let f = find(&d[0]);
    assert_eq!((f["query"].as_str(), f["focused"].as_bool()), (Some(""), Some(true)), "{}", d[0]);
    // Typed: three hits, the one in view current and highlighted.
    let f = find(&d[1]);
    assert_eq!(f["query"], "nee", "{}", d[1]);
    assert_eq!(f["hits"], 3, "{}", d[1]);
    assert_eq!(f["count"], "3 of 3", "{}", d[1]);
    assert_eq!(f["current_row"], 4, "{}", d[1]);
    assert_eq!(f["highlights"], 1, "{}", d[1]);
    assert_eq!(offset(&d[1]), 0, "{}", d[1]);
    // Next wraps to the first, in history: the view moves to it.
    let f = find(&d[2]);
    assert_eq!(f["count"], "1 of 3", "{}", d[2]);
    assert!(offset(&d[2]) > 0, "{}", d[2]);
    assert!(f["current_row"].is_u64(), "{}", d[2]);
    assert!(screen(&d[2]).contains("row 5 needle"), "{}", d[2]);
    // Previous wraps back to the last.
    let f = find(&d[3]);
    assert_eq!(f["count"], "3 of 3", "{}", d[3]);
    assert!(screen(&d[3]).contains("row 45 needle"), "{}", d[3]);
    // Return in the field is Find Next.
    let f = find(&d[4]);
    assert_eq!(f["count"], "1 of 3", "{}", d[4]);
    // A new query: "row 2" is rows 2 and 20..29.
    let f = find(&d[5]);
    assert_eq!(f["hits"], 11, "{}", d[5]);
    // Esc closes the bar; the view stays where it is.
    assert!(d[6]["find"].is_null(), "{}", d[6]);
    assert_eq!(offset(&d[6]), offset(&d[5]), "{}", d[6]);
}

/// The find bar keeps searching across a switch to the alternate screen
/// and back (vim or less opening and closing): the hits are the screen
/// being shown's, found again at each switch, and the current one is
/// highlighted.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_find_bar_restarts_on_the_alternate_screen() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("find-alt");
    let wait = "i=0; while [ \"$(stty size)\" != '20 80' ] && [ $i -lt 100 ]; do sleep 0.05; i=$((i+1)); done; ";
    // Two hits on the main screen, three on the alternate one.
    let main = "echo 'one needle'; echo 'two needle'; ";
    let enter = "printf '\\033[?1049h\\033[Hneedle needle\\r\\nneedle'; ";
    let leave = "printf '\\033[?1049l'; ";
    let script = format!(
        "{resize}{wait}{main}{find}{dump}{enter}sleep 1; {dump}{leave}sleep 1; {dump}{quit}sleep 20",
        resize = osc("resize 80 20"),
        find = [osc("menu find"), osc("find needle")].concat(),
        dump = dump_and_wait(&dir),
        quit = osc("quit"),
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 3, "{d:?}");
    let hits = |i: usize| d[i]["find"]["hits"].clone();
    assert_eq!(hits(0), 2, "{}", d[0]);
    assert_eq!(hits(1), 3, "{}", d[1]);
    assert!(d[1]["find"]["current"].is_u64(), "{}", d[1]);
    assert_eq!(d[1]["find"]["highlights"], 3, "{}", d[1]);
    assert_eq!(hits(2), 2, "{}", d[2]);
    assert!(d[2]["find"]["current"].is_u64(), "{}", d[2]);
    assert_eq!(d[2]["find"]["highlights"], 2, "{}", d[2]);
}

/// Clicking in the find bar of a pane that isn't focused focuses that
/// pane, and the keys stay in the bar's field.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_click_in_a_find_bar_focuses_its_pane() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("find-click");
    let script = format!(
        "sleep 1; {split}sleep 1; {first}{find}{second}sleep 1; {dump}{click}sleep 1; {dump}{quit}sleep 20",
        split = osc("split right"),
        first = osc("focus 0"),
        find = osc("menu find"),
        second = osc("focus 1"),
        click = osc("find-click 0"),
        dump = dump_and_wait(&dir),
        quit = osc("quit"),
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 2, "{d:?}");
    let focused = |d: &Value| d["panes"].as_array().unwrap().iter().position(|p| p["focused"] == true);
    // The other pane has the keys; its bar isn't the focused pane's.
    assert_eq!(focused(&d[0]), Some(1), "{}", d[0]);
    assert!(d[0]["find"].is_null(), "{}", d[0]);
    // The click gave the bar's pane the focus, and the field the keys.
    assert_eq!(focused(&d[1]), Some(0), "{}", d[1]);
    assert_eq!(d[1]["find"]["focused"], true, "{}", d[1]);
}

/// `images = true`: a kitty PNG sent by the program is listed in the
/// dump's `images` section, decoded, placed in view and counted in the
/// pool (docs/design/images.md §11).
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_kitty_png_is_listed_in_the_images_dump() {
    if !have_metal() {
        return;
    }
    // A 1 x 1 RGBA PNG.
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let dir = TempDir::new("images");
    std::fs::create_dir_all(dir.config_home().join("terminal-decline")).unwrap();
    std::fs::write(dir.config_home().join("terminal-decline/config.toml"), "images = true\n").unwrap();
    // Some text too: every dump checks glyphs were drawn. Decoding runs on
    // a queue: give it a moment before the dump.
    let script = format!(
        "printf 'images\\033_Ga=T,f=100,i=7,q=2;{PNG}\\033\\\\\\n'; sleep 1; {}{}sleep 20",
        osc("dump"),
        osc("quit")
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 1, "{d:?}");
    check_dump(&d[0]);
    let images = &d[0]["images"];
    assert_eq!(images["enabled"], true, "{}", d[0]);
    assert_eq!(images["count"], 1, "{}", d[0]);
    assert_eq!(images["ready"], 1, "{}", d[0]);
    assert_eq!(images["placements_in_view"], 1, "{}", d[0]);
    assert!(num(images, "bytes") > 0.0, "{}", d[0]);
    assert!(num(images, "budget") >= (32 << 20) as f64, "{}", d[0]);
}

/// `images = true`: an iTerm2 image (`imgcat`'s OSC 1337 `File=`) and a
/// Sixel image are listed in the dump's `images` section, both decoded
/// (docs/design/images.md §11).
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_iterm2_and_sixel_images_are_listed_in_the_images_dump() {
    if !have_metal() {
        return;
    }
    // A 1 x 1 RGBA PNG.
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    // A 4 x 6 red Sixel image.
    const SIXEL: &str = "\\033P0;1;0q\"1;1;4;6#1;2;100;0;0#1!4~\\033\\\\";
    let dir = TempDir::new("images-own");
    std::fs::create_dir_all(dir.config_home().join("terminal-decline")).unwrap();
    std::fs::write(dir.config_home().join("terminal-decline/config.toml"), "images = true\n").unwrap();
    // Some text too: every dump checks glyphs were drawn. Decoding runs on
    // a queue: give it a moment before the dump.
    let script = format!(
        "printf 'images\\n\\033]1337;File=inline=1:{PNG}\\007\\n{SIXEL}\\n'; sleep 1; {}{}sleep 20",
        osc("dump"),
        osc("quit")
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 1, "{d:?}");
    check_dump(&d[0]);
    let images = &d[0]["images"];
    assert_eq!(images["enabled"], true, "{}", d[0]);
    assert_eq!(images["count"], 2, "{}", d[0]);
    assert_eq!(images["ready"], 2, "{}", d[0]);
    assert!(num(images, "bytes") > 0.0, "{}", d[0]);
}

/// A memory-pressure warning (simulated with the control's
/// `memory-pressure`, which runs the same handler) gives memory back:
/// history shrinks (its blocks compressed), and the window draws the
/// same screen again afterwards (roadmap R.5).
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_memory_pressure_gives_memory_back_and_redraws() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("pressure");
    let script = format!(
        "seq 1 100000 | sed 's/^/2026-10-30T09:00:00Z INFO request served path=\\/api\\/v1\\/items id=/'; echo done; {}{}{}{}sleep 20",
        osc("dump"),
        osc("memory-pressure"),
        osc("dump"),
        osc("quit")
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 2, "{d:?}");
    for x in &d {
        check_dump(x);
    }
    assert_eq!(num(&d[1], "memory_reliefs"), num(&d[0], "memory_reliefs") + 1.0);
    let history = |x: &Value| num(&x["panes"][0], "history_bytes");
    assert!(history(&d[1]) < history(&d[0]), "history {} -> {}", history(&d[0]), history(&d[1]));
    // Drawn again, the same.
    assert!(num(&d[1], "frames") > num(&d[0], "frames"));
    assert_eq!(screen(&d[1]), screen(&d[0]));
    assert_eq!(num(&d[1], "glyphs"), num(&d[0], "glyphs"));
    eprintln!(
        "footprint {:.1} -> {:.1} MB, history {:.2} -> {:.2} MB",
        num(&d[0], "phys_footprint") / 1e6,
        num(&d[1], "phys_footprint") / 1e6,
        history(&d[0]) / 1e6,
        history(&d[1]) / 1e6
    );
}

/// Terminal Decline > Edit config.toml (the `edit-config` control command)
/// opens a tab running the user's editor on the config file, never
/// TextEdit: a fake editor named by the `editor` setting writes the file it
/// was given into a marker. The window gets a second tab for it.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_edit_config_opens_a_tab_running_the_editor() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("editor");
    let d = dir.path().display();
    let conf_dir = dir.config_home().join("terminal-decline");
    std::fs::create_dir_all(&conf_dir).unwrap();
    let file = conf_dir.join("config.toml");
    let fake = dir.path().join("fake-editor");
    std::fs::write(
        &fake,
        format!("#!/bin/sh\nprintf %s \"$1\" > '{d}/marker.tmp'\nmv '{d}/marker.tmp' '{d}/marker'\nsleep 30\n"),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(&file, format!("editor = '{}'\n", fake.display())).unwrap();
    let script = format!(
        "echo before; {edit}i=0; until [ -e '{d}/marker' ] || [ $i -ge 200 ]; do sleep 0.05; i=$((i+1)); done; \
         echo edited; {dump}{wait}{quit}sleep 20",
        edit = osc("edit-config"),
        dump = osc("dump"),
        wait = dump_until(&dir, "edited[|]"),
        quit = osc("quit"),
    );
    let mut cmd = Command::new(APP);
    cmd.args(["-e", "/bin/sh", "-c", &script])
        .env("XDG_CONFIG_HOME", dir.config_home())
        .env("DECLINE_TEST_DUMP", dir.dump_file())
        // Their login shell, as the app runs the editor through it.
        .env("SHELL", "/bin/sh")
        .current_dir(dir.path());
    let (status, _) = run(&mut cmd);
    assert!(status.success(), "{status}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("marker")).unwrap_or_default(),
        file.display().to_string()
    );
    let all = dumps(&dir);
    let last = all.iter().find(|x| screen(x).contains("edited|")).expect("no dump after the edit");
    check_dump(last);
    assert_eq!(num(last, "tabs"), 2.0, "the editor has its own tab: {last}");
}

/// `ctl` against an app started to host windows: a socket directory and
/// HOME of the test's own, a short path (a socket's path must fit in 104
/// bytes).
struct CtlApp {
    root: PathBuf,
}

impl CtlApp {
    fn new(name: &str) -> CtlApp {
        let root = PathBuf::from(format!("/tmp/tdw-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("home/.config/terminal-decline")).unwrap();
        std::fs::write(
            root.join("home/.config/terminal-decline/config.toml"),
            "padding = 0\nfont_size = 13\n",
        )
        .unwrap();
        CtlApp { root }
    }

    /// Run `ctl args`: its exit status and JSON.
    fn run(&self, args: &[&str]) -> (i32, Value) {
        let out = Command::new(APP)
            .arg("ctl")
            .args(args)
            .env("TERMINAL_DECLINE_CTL_DIR", self.root.join("s"))
            .env("TERMINAL_DECLINE_CTL_IDLE_MS", "4000")
            .env("HOME", self.root.join("home"))
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("DECLINE_TEST_DUMP")
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let v = serde_json::from_str(&text).unwrap_or_else(|e| panic!("ctl {args:?}: {e}: {text}"));
        (out.status.code().unwrap_or(-1), v)
    }

    fn ok(&self, args: &[&str]) -> Value {
        let (code, v) = self.run(args);
        assert_eq!((code, &v["ok"]), (0, &Value::Bool(true)), "ctl {args:?}: {v}");
        v
    }
}

impl Drop for CtlApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn process_exists(pid: u64) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

/// `ctl spawn --window` with no app running starts one in the background
/// that hosts the window and serves every `ctl` command, headless sessions
/// too; killing the session closes the window, a program that exits ends
/// the session, and the app leaves once it is idle
/// (docs/design/agent-api.md §6).
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_ctl_windows_are_sessions_in_a_background_app() {
    if !have_metal() {
        return;
    }
    let ctl = CtlApp::new("win");
    let w = ctl.ok(&["spawn", "--window", "--name", "w", "--size", "60x10", "--", "/bin/sh"]);
    assert_eq!(w["session"], "w");
    let hello = ctl.ok(&["hello"]);
    assert_eq!(
        (hello["server"].as_str(), hello["windows"].as_bool()),
        (Some("app"), Some(true)),
        "{hello}"
    );
    let app = hello["pid"].as_u64().unwrap();
    assert_ne!(app, std::process::id() as u64);
    assert_eq!(ctl.ok(&["list"])["sessions"][0]["window"], true);

    // Driven like a headless session.
    ctl.ok(&["send", "w", "--text", "PS1='$ '; echo $((6*7)); stty size\n"]);
    ctl.ok(&["wait-for", "w", "--regex", "(?m)^42$", "--timeout", "10000"]);
    ctl.ok(&["wait-for", "w", "--text", "10 60", "--timeout", "10000"]);
    let screen = ctl.ok(&["screen", "w", "--json"]);
    assert_eq!((screen["cols"].as_u64(), screen["rows"].as_u64()), (Some(60), Some(10)), "{screen}");
    ctl.ok(&["resize", "w", "50x8"]);
    ctl.ok(&["send", "w", "--text", "stty size\n"]);
    ctl.ok(&["wait-for", "w", "--text", "8 50", "--timeout", "10000"]);
    let png = ctl.root.join("w.png");
    ctl.ok(&["screenshot", "w", png.to_str().unwrap()]);
    assert!(std::fs::metadata(&png).unwrap().len() > 100);

    // The same app serves headless sessions beside it.
    let h = ctl.ok(&["spawn", "--name", "h", "--", "/bin/sh"]);
    assert_eq!(h["session"], "h");
    assert_eq!(ctl.ok(&["list"])["sessions"].as_array().unwrap().len(), 2);

    // A program that exits ends the session with its status.
    ctl.ok(&["send", "w", "--text", "exit 7\n"]);
    let done = ctl.ok(&["wait-for", "w", "--exit", "--timeout", "10000"]);
    assert_eq!(done["exit"]["code"], 7, "{done}");

    // Killing a session closes its window and hangs up on the program.
    ctl.ok(&["spawn", "--window", "--name", "k", "--", "/bin/sh"]);
    let pid = ctl.ok(&["stats", "k"])["pid"].as_u64().unwrap();
    ctl.ok(&["kill", "k"]);
    let until = Instant::now() + Duration::from_secs(10);
    while process_exists(pid) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!process_exists(pid), "the program is still running");

    // A program that can't start is an error, not a window.
    let (code, v) = ctl.run(&["spawn", "--window", "--name", "bad", "--", "/no/such/program"]);
    assert_eq!(code, 1, "{v}");

    // Nothing running and nobody asking: the app leaves.
    ctl.ok(&["kill", "h"]);
    let until = Instant::now() + Duration::from_secs(20);
    while process_exists(app) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!process_exists(app), "the hosting app is still running");
}

/// A window session with `--shell` marks its commands as a headless one
/// does: `wait-for --command` gets the status and duration.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_ctl_window_sessions_mark_commands() {
    if !have_metal() {
        return;
    }
    let ctl = CtlApp::new("cmd");
    // Its history stays out of the checkout the integration is read from.
    let history = format!("HISTFILE={}", ctl.root.join("history").display());
    ctl.ok(&[
        "spawn",
        "--window",
        "--name",
        "sh",
        "--shell",
        "--env",
        "SHELL=/bin/zsh",
        "--env",
        &history,
    ]);
    ctl.ok(&["wait-for", "sh", "--idle", "700", "--timeout", "15000"]);
    // No history: zsh would save it in the checkout the integration is
    // read from.
    ctl.ok(&["send", "sh", "--text", "unset HISTFILE\n"]);
    ctl.ok(&["wait-for", "sh", "--idle", "700", "--timeout", "15000"]);
    ctl.ok(&["send", "sh", "--text", "sleep 1; (exit 4)\n"]);
    let w = ctl.ok(&["wait-for", "sh", "--command", "--timeout", "10000"]);
    assert_eq!(w["status"], 4, "{w}");
    assert!(w["duration_ms"].as_u64().unwrap() >= 800, "{w}");
    ctl.ok(&["kill", "sh"]);
}

/// A command that exited non-zero has its prompt's row marked in the
/// frame (the red mark in the padding); Jump to Previous Failed Command
/// scrolls to it, past a prompt whose command succeeded, and no further.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_failed_commands_are_marked_and_jumped_to() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("failed");
    let wait = "i=0; while [ \"$(stty size)\" != '20 80' ] && [ $i -lt 100 ]; do sleep 0.05; i=$((i+1)); done; ";
    // Prompts at lines 0, 41 and 82 and the last at 123; `two` exits 3.
    let session = "s() { printf '\\033]133;A\\007$ %s\\033]133;B\\007\\n\\033]133;C\\007' \"$1\"; \
                   i=0; while [ $i -lt 40 ]; do echo \"$1 $i\"; i=$((i+1)); done; printf '\\033]133;D;%s\\007' \"$2\"; }; \
                   s one 0; s two 3; s three 0; printf '\\033]133;A\\007$ '; ";
    let dump = osc("dump");
    let back = osc("menu previous_failed");
    let script = format!(
        "{resize}{wait}{session}{dump}{back}{dump}{back}{dump}{next}{dump}{quit}sleep 20",
        resize = osc("resize 80 20"),
        next = osc("menu next_failed"),
        quit = osc("quit"),
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 4, "{d:?}");
    let offset = |x: &Value| num(x, "view_offset") as u64;
    let top = |x: &Value| screen(x).lines().next().unwrap_or_default().to_string();
    let failed = |x: &Value| x["failed_rows"].clone();
    // At the bottom (lines 104..124) no failed prompt is in view.
    assert_eq!((offset(&d[0]), failed(&d[0])), (0, serde_json::json!([])), "{}", d[0]);
    // Past `three` (succeeded) to `two`, marked at the top row.
    assert_eq!((offset(&d[1]), top(&d[1])), (63, "$ two|".into()), "{}", d[1]);
    assert_eq!(failed(&d[1]), serde_json::json!([0]), "{}", d[1]);
    // Nothing failed above it, nor below: the view stays.
    assert_eq!(offset(&d[2]), 63, "{}", d[2]);
    assert_eq!(offset(&d[3]), 63, "{}", d[3]);
}

/// Copy Last Command and Copy Last Command's Output put the last
/// command line run and its output on the clipboard (listed in the dump
/// under test control), wherever the view is scrolled, selecting nothing.
#[test]
#[ignore = "opens a window; run with --run-ignored"]
fn smoke_copy_last_command_and_its_output() {
    if !have_metal() {
        return;
    }
    let dir = TempDir::new("copylast");
    let wait = "i=0; while [ \"$(stty size)\" != '20 80' ] && [ $i -lt 100 ]; do sleep 0.05; i=$((i+1)); done; ";
    let session = "s() { printf '\\033]133;A\\007$ \\033]133;B\\007%s\\n\\033]133;C\\007' \"$1\"; \
                   i=0; while [ $i -lt 30 ]; do echo \"$1 $i\"; i=$((i+1)); done; printf '\\033]133;D;0\\007'; }; \
                   s 'echo one'; s 'echo two'; printf '\\033]133;A\\007$ \\033]133;B\\007typing'; ";
    let script = format!(
        "{resize}{wait}{session}{command}{output}{up}{output}{dump}{quit}sleep 20",
        resize = osc("resize 80 20"),
        command = osc("menu copy_command"),
        output = osc("menu copy_output"),
        up = osc("key 126 cmd"),
        dump = osc("dump"),
        quit = osc("quit"),
    );
    let (status, _) = decline(&dir, &script, true);
    assert!(status.success(), "{status}");
    let d = dumps(&dir);
    assert_eq!(d.len(), 1, "{d:?}");
    let output: Vec<String> = (0..30).map(|i| format!("echo two {i}")).collect();
    let output = output.join("\n");
    assert_eq!(d[0]["copied"], serde_json::json!(["echo two", output, output]), "{}", d[0]);
    assert!(num(&d[0], "view_offset") > 0.0, "{}", d[0]);
    assert!(d[0]["selection"].is_null(), "{}", d[0]);
}
