//! Headless pipeline tests: real programs on a real PTY through the same
//! `session` module the app uses (reader and writer threads, the shared
//! terminal, the dirty flag and wake-ups), with no window. Keys go through
//! `keys::translate`, as in the app; a fake draw loop drives
//! `frame_policy::decide` for the concurrency cases.
//!
//! Every shell is `/bin/sh` with HOME set to a fresh temp dir, so the user's
//! shell and dotfiles don't matter. Waits are on wake-ups with a deadline,
//! and a timeout prints the last screen.
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use decline::frame_policy::{self, Decision};
use decline::keys::{self, KeyAction, KeyInput, KeyModes, flags};
use decline::session::{Hooks, Outcome, Session};
use term_core::{Attrs, ColorKind, Term, Terminal};
use term_pty::{Command, Size};

/// Upper bound for any single wait.
const DEADLINE: Duration = Duration::from_secs(15);
const PROMPT: &str = "READY>";

fn size(cols: u16, lines: u16) -> Size {
    Size {
        cols,
        lines,
        cell_width: 8,
        cell_height: 16,
    }
}

/// A temp directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("decline-pipeline-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir.canonicalize().unwrap())
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What the reader thread's callbacks have done.
#[derive(Default)]
struct Events {
    wakes: u64,
    exits: u64,
    outcome: Option<Outcome>,
}

#[derive(Default)]
struct Signal {
    events: Mutex<Events>,
    cv: Condvar,
}

impl Signal {
    fn hooks(self: &Arc<Signal>) -> Hooks {
        let (a, b) = (self.clone(), self.clone());
        Hooks::new(
            Arc::new(move || {
                a.events.lock().unwrap().wakes += 1;
                a.cv.notify_all();
            }),
            Box::new(move |outcome| {
                let mut ev = b.events.lock().unwrap();
                ev.exits += 1;
                ev.outcome = Some(outcome);
                drop(ev);
                b.cv.notify_all();
            }),
        )
    }

    fn wakes(&self) -> u64 {
        self.events.lock().unwrap().wakes
    }

    fn outcome(&self) -> Option<Outcome> {
        self.events.lock().unwrap().outcome
    }

    fn exits(&self) -> u64 {
        self.events.lock().unwrap().exits
    }

    /// Wait until the wake count passes `seen` or the program exits, or
    /// `until`.
    fn wait_past(&self, seen: u64, until: Instant) {
        let mut ev = self.events.lock().unwrap();
        while ev.wakes == seen && ev.exits == 0 {
            let now = Instant::now();
            if now >= until {
                return;
            }
            ev = self.cv.wait_timeout(ev, until - now).unwrap().0;
        }
    }

    fn wait_exit(&self, within: Duration) -> bool {
        let until = Instant::now() + within;
        let mut ev = self.events.lock().unwrap();
        while ev.exits == 0 {
            let now = Instant::now();
            if now >= until {
                return false;
            }
            ev = self.cv.wait_timeout(ev, until - now).unwrap().0;
        }
        true
    }
}

struct Harness {
    session: Option<Session>,
    signal: Arc<Signal>,
    _home: TempDir,
}

/// The command for `argv` (empty: an interactive shell) under `/bin/sh`
/// with HOME at `home`, prompting with [`PROMPT`].
fn command(argv: &[&str], home: &Path) -> Command {
    std::fs::write(home.join(".profile"), format!("PS1='{PROMPT} '\nexport PS1\n")).unwrap();
    Command {
        argv: argv.iter().map(|s| s.to_string()).collect(),
        cwd: Some(home.to_str().unwrap().into()),
        env: vec![
            ("HOME".into(), home.to_str().unwrap().into()),
            ("BASH_SILENCE_DEPRECATION_WARNING".into(), "1".into()),
            ("ENV".into(), String::new()),
            ("LESSHISTFILE".into(), "-".into()),
            ("LESS".into(), String::new()),
        ],
        shell: Some("/bin/sh".into()),
    }
}

impl Harness {
    /// Run `argv` (empty: an interactive shell); `setup` can write files
    /// into the home dir first.
    fn start(name: &str, argv: &[&str], setup: impl FnOnce(&Path)) -> Harness {
        Harness::start_sized(name, argv, size(80, 24), setup)
    }

    fn start_sized(name: &str, argv: &[&str], sz: Size, setup: impl FnOnce(&Path)) -> Harness {
        let home = TempDir::new(name);
        setup(home.path());
        let cmd = command(argv, home.path());
        let signal = Arc::new(Signal::default());
        let term = Term::new(sz.cols as usize, sz.lines as usize, 1000);
        let session = Session::spawn(&cmd, sz, term, signal.hooks()).expect("spawn");
        Harness {
            session: Some(session),
            signal,
            _home: home,
        }
    }

    fn shell(name: &str) -> Harness {
        let h = Harness::start(name, &[], |_| {});
        h.wait_for("first prompt", |d| prompts(d) == 1);
        h
    }

    fn session(&self) -> &Session {
        self.session.as_ref().unwrap()
    }

    fn dump(&self) -> String {
        self.session().term().lock().unwrap().state.dump()
    }

    /// Wait until `pred` holds for the terminal, re-checking after each
    /// wake-up. Like a frame, each check takes the dirty flag first, so the
    /// next output wakes us again.
    fn wait_term<T>(&self, what: &str, pred: impl Fn(&Terminal) -> Option<T>) -> T {
        let until = Instant::now() + DEADLINE;
        loop {
            let seen = self.signal.wakes();
            self.session().take_dirty();
            let exited = self.signal.exits() > 0;
            {
                let t = self.session().term().lock().unwrap();
                if let Some(v) = pred(&t.state) {
                    return v;
                }
                if Instant::now() >= until || exited && seen == self.signal.wakes() && !self.session().take_dirty() {
                    panic!(
                        "timed out waiting for {what}{}; screen:\n{}",
                        if exited { " (program exited)" } else { "" },
                        t.state.dump()
                    );
                }
            }
            if !exited {
                self.signal.wait_past(seen, until);
            }
        }
    }

    fn wait_for(&self, what: &str, pred: impl Fn(&str) -> bool) -> String {
        self.wait_term(what, |t| {
            let d = t.dump();
            pred(&d).then_some(d)
        })
    }

    fn modes(&self) -> KeyModes {
        let t = self.session().term().lock().unwrap();
        KeyModes {
            app_cursor: t.state.modes().app_cursor,
            app_keypad: t.state.modes().app_keypad,
            kitty_flags: t.state.kitty_keyboard_flags(),
            ..KeyModes::default()
        }
    }

    /// A key press through `keys::translate`. Text that goes to the input
    /// system is sent as typed, as `insertText` would.
    fn press(&self, input: KeyInput) {
        match keys::translate(&input, self.modes()) {
            KeyAction::Send(b) => self.session().send(b),
            KeyAction::ToTextInput => self.session().send(input.chars.into_bytes()),
            KeyAction::ToMenu => panic!("{input:?} went to the menus"),
            KeyAction::Ignore => {}
        }
    }

    fn key(&self, keycode: u16) {
        self.press(KeyInput {
            keycode,
            ..KeyInput::default()
        });
    }

    fn ctrl(&self, c: char) {
        self.press(char_key(c, flags::CONTROL));
    }

    /// Type text; `\n` is the Return key.
    fn type_text(&self, text: &str) {
        for c in text.chars() {
            if c == '\n' {
                self.key(RETURN);
            } else {
                self.press(char_key(c, 0));
            }
        }
    }

    fn resize(&self, cols: u16, lines: u16) {
        self.session().resize(size(cols, lines)).unwrap();
    }

    /// Wait for the program to exit, then for the reader thread (re-raising
    /// its panic, if any).
    fn wait_exit(&mut self) {
        assert!(self.signal.wait_exit(DEADLINE), "program didn't exit; screen:\n{}", self.dump());
        self.session.as_mut().unwrap().wait();
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(s) = self.session.take()
            && !std::thread::panicking()
        {
            s.shut_down();
        }
    }
}

const RETURN: u16 = 0x24;
const ESCAPE: u16 = 0x35;
const BACKSPACE: u16 = 0x33;
const UP: u16 = 0x7E;
const DOWN: u16 = 0x7D;

fn char_key(c: char, flags: usize) -> KeyInput {
    KeyInput {
        keycode: 0,
        chars: c.to_string(),
        chars_ignoring_mods: c.to_string(),
        chars_shift_only: c.to_string(),
        flags,
        ..KeyInput::default()
    }
}

fn prompts(dump: &str) -> usize {
    dump.lines().filter(|l| l.starts_with(PROMPT)).count()
}

fn has_line(dump: &str, line: &str) -> bool {
    dump.lines().any(|l| l.trim_end_matches(['|', '⏎', '·']) == line)
}

fn status(dump: &str) -> &str {
    dump.lines().last().unwrap()
}

fn have(program: &str) -> bool {
    std::process::Command::new("/bin/sh")
        .args(["-c", &format!("command -v {program}")])
        .stdout(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn python_with_curses() -> bool {
    std::process::Command::new("python3")
        .args(["-c", "import curses, termios, tty"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

// --- Shells -----------------------------------------------------------------

#[test]
fn shell_prompt_and_output() {
    let h = Harness::shell("prompt");
    h.type_text("echo hello-$((40+2))\n");
    let d = h.wait_for("command output", |d| has_line(d, "hello-42") && prompts(d) == 2);
    assert!(d.starts_with("READY> echo hello-$((40+2))|\nhello-42|\nREADY>·|"), "{d}");
    assert!(status(&d).starts_with("cursor=(2,7) alt=false"), "{d}");
}

#[test]
fn typing_through_translate_with_line_editing() {
    let h = Harness::shell("typing");
    // "echo hx", Backspace, "i", Return.
    h.type_text("echo hx");
    h.key(BACKSPACE);
    h.type_text("i\n");
    h.wait_for("hi", |d| has_line(d, "hi") && prompts(d) == 2);
}

#[test]
fn ctrl_c_interrupts_sleep() {
    let h = Harness::shell("ctrlc");
    h.type_text("echo sleeping; sleep 100\n");
    h.wait_for("sleep to start", |d| has_line(d, "sleeping"));
    // The shell may still be starting sleep; Ctrl+C until the prompt is back.
    let until = Instant::now() + DEADLINE;
    while prompts(&h.dump()) < 2 {
        assert!(Instant::now() < until, "Ctrl+C didn't stop sleep:\n{}", h.dump());
        h.ctrl('c');
        let seen = h.signal.wakes();
        h.session().take_dirty();
        h.signal.wait_past(seen, Instant::now() + Duration::from_millis(500));
    }
    h.type_text("echo after\n");
    h.wait_for("the next command", |d| has_line(d, "after"));
}

#[test]
fn exit_status_and_on_exit() {
    let mut h = Harness::shell("exit");
    h.type_text("sh -c 'exit 3'; echo status=$?\n");
    h.wait_for("status", |d| has_line(d, "status=3"));
    assert_eq!(h.signal.exits(), 0);
    h.type_text("exit 7\n");
    h.wait_exit();
    assert_eq!(h.signal.exits(), 1, "on_exit fires once");
}

/// Titles and bells reach `on_events` as they are parsed, with no frame
/// drawn and the dirty flag never taken: a background tab is never drawn,
/// but its title shows in the tab bar.
#[test]
fn events_arrive_without_drawing() {
    let home = TempDir::new("events");
    let script = "printf '\\033]2;first\\007'; sleep 0.2; printf '\\033]0;second\\007\\007'; sleep 5";
    let cmd = command(&["/bin/sh", "-c", script], home.path());
    let (tx, rx) = std::sync::mpsc::channel();
    let mut hooks = Arc::new(Signal::default()).hooks();
    hooks.on_events = Some(Box::new(move |e| tx.send(e).unwrap()));
    let session = Session::spawn(&cmd, size(80, 24), Term::new(80, 24, 100), hooks).expect("spawn");
    let mut got = Vec::new();
    while !got.iter().any(|e: &decline::session::Events| e.bell) {
        got.push(rx.recv_timeout(DEADLINE).expect("no events"));
    }
    let titles: Vec<_> = got.iter().filter_map(|e| e.title.as_deref()).collect();
    assert_eq!(titles, ["first", "second"], "{got:?}");
    assert!(got.iter().all(|e| !e.is_empty()), "{got:?}");
    // Taken out of the terminal, so a frame doesn't act on them again.
    assert_eq!(session.lock_term().state.take_title(), None);
    session.shut_down();
}

// --- Failures shown in the window -------------------------------------------

/// Run `argv` on a `cols`-wide terminal until the session ends; the outcome
/// and the final screen.
fn run_to_end(name: &str, argv: &[&str], cols: u16) -> (Outcome, String) {
    let mut h = Harness::start_sized(name, argv, size(cols, 24), |_| {});
    h.wait_exit();
    (h.signal.outcome().unwrap(), h.dump())
}

#[test]
fn quick_failure_keeps_the_window_with_its_status() {
    let (outcome, screen) = run_to_end("exit3", &["/bin/sh", "-c", "echo oops; exit 3"], 80);
    assert_eq!(outcome, Outcome::Hold);
    assert!(has_line(&screen, "oops"), "{screen}");
    assert!(has_line(&screen, "[process exited with status 3]"), "{screen}");
    assert!(has_line(&screen, "press any key to close"), "{screen}");
}

#[test]
fn quick_success_closes() {
    let (outcome, screen) = run_to_end("exit0", &["/bin/sh", "-c", "echo fine; exit 0"], 80);
    assert_eq!(outcome, Outcome::Closed);
    assert!(!screen.contains("[process exited"), "{screen}");
}

#[test]
fn failure_after_a_second_closes() {
    let (outcome, screen) = run_to_end("slow-exit3", &["/bin/sh", "-c", "sleep 1.2; exit 3"], 80);
    assert_eq!(outcome, Outcome::Closed);
    assert!(!screen.contains("[process exited"), "{screen}");
}

/// `decline -e /nonexistent`: the shell says why and exits with 127.
#[test]
fn missing_program_keeps_the_window() {
    let (outcome, screen) = run_to_end("missing", &["/nonexistent"], 80);
    assert_eq!(outcome, Outcome::Hold);
    assert!(screen.contains("/nonexistent"), "{screen}");
    // 127 from sh; bash (which /bin/sh is on macOS) says 126.
    assert!(
        has_line(&screen, "[process exited with status 127]") || has_line(&screen, "[process exited with status 126]"),
        "{screen}"
    );
}

/// A command that can't even be started (a NUL byte can't be passed to
/// exec): no program, the error in the terminal, and on_exit right away.
#[test]
fn program_that_cannot_start_says_why() {
    let home = TempDir::new("nostart");
    let cmd = command(&["bad\0name"], home.path());
    let signal = Arc::new(Signal::default());
    let session = Session::spawn(&cmd, size(80, 24), Term::new(80, 24, 100), signal.hooks()).expect("spawn");
    assert_eq!(signal.exits(), 1);
    assert_eq!(signal.outcome(), Some(Outcome::Hold));
    assert_eq!(session.pid(), None);
    let screen = session.lock_term().state.dump();
    assert!(
        screen.starts_with("decline: could not start \"bad\\0name\": nul byte in argument|"),
        "{screen}"
    );
    assert!(has_line(&screen, "press any key to close"), "{screen}");
    // Typing into it and resizing it do nothing harmful.
    session.send(b"x".to_vec());
    session.resize(size(60, 20)).unwrap();
    session.shut_down();
}

/// Config errors go into the terminal (dim red) before the program's output.
#[test]
fn config_errors_show_before_the_output() {
    let home = TempDir::new("badconfig");
    let file = home.path().join("config.toml");
    std::fs::write(&file, "font_sise = 3\nfont_size = 15\n").unwrap();
    let (config, errors) = decline::config::load(&[file.as_path()]);
    assert_eq!(config.font_size, 15.0);
    assert_eq!(errors.len(), 1, "{errors:?}");
    let cmd = command(&["/bin/sh", "-c", "echo program output; sleep 0.3"], home.path());
    let signal = Arc::new(Signal::default());
    let mut term = Term::new(300, 24, 100);
    for e in &errors {
        decline::session::write_notice(&mut term, e);
    }
    let mut h = Harness {
        session: Some(Session::spawn(&cmd, size(300, 24), term, signal.clone().hooks()).unwrap()),
        signal,
        _home: home,
    };
    h.wait_for("the output", |d| has_line(d, "program output"));
    let lines: Vec<String> = h.dump().lines().map(String::from).collect();
    // The notice lists every setting, so it may wrap onto more rows
    // (marked ⏎ at the end of each but its last).
    let rows = lines.iter().position(|l| !l.ends_with('⏎')).unwrap() + 1;
    assert!(lines[0].starts_with("decline: config: font_sise: unknown field"), "{lines:?}");
    assert!(
        lines[rows - 1].trim_end_matches(['|', '⏎', '·']).ends_with("; ignored"),
        "{lines:?}"
    );
    assert!(lines[rows].starts_with("program output"), "{lines:?}");
    {
        let t = h.session().lock_term();
        let row = t.state.visible_row(0);
        let cell = row.cell(0);
        assert!(
            cell.style().attrs.contains(Attrs::DIM) && cell.style().fg.kind() == ColorKind::Indexed(1),
            "{cell:?}"
        );
    }
    h.wait_exit();
    assert_eq!(h.signal.outcome(), Some(Outcome::Closed));
}

/// A panic while parsing one session's output (forced through the
/// `test-hooks` feature) stops that session only: it shows where the crash
/// log is, its program is killed, its terminal's lock isn't poisoned, and
/// another session keeps working.
#[test]
fn parser_panic_stops_only_that_session() {
    let logs = TempDir::new("crash-logs");
    decline::crash::install(logs.path().to_path_buf(), false);

    let other = Harness::shell("survivor");
    let script = "printf 'before the bug\\n'; printf '\\033]7770;test-panic\\007'; echo after; sleep 30";
    let mut broken = Harness::start_sized("panic", &["/bin/sh", "-c", script], size(300, 24), |_| {});
    let pid = broken.session().pid().unwrap();
    broken.wait_exit();
    assert_eq!(broken.signal.outcome(), Some(Outcome::Hold));
    assert!(!broken.session().term().is_poisoned());

    // Other tests failing in this process would leave logs here too.
    let (log, text) = std::fs::read_dir(logs.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .map(|p| (p.clone(), std::fs::read_to_string(p).unwrap()))
        .find(|(_, text)| text.contains("test-hooks: the parse step was asked to panic"))
        .expect("a crash log");
    assert!(text.contains(concat!("decline ", env!("CARGO_PKG_VERSION"))), "{text}");
    assert!(text.contains("thread 'pty-reader'") && text.contains("backtrace:"), "{text}");

    let screen = broken.dump();
    let expected = format!(
        "decline hit a bug and stopped this session; details in {}",
        decline::crash::tilde(&log)
    );
    assert!(has_line(&screen, &expected), "{screen}");
    assert!(has_line(&screen, "press any key to close"), "{screen}");
    assert!(!screen.contains("after"), "{screen}");

    // The program was killed: only a zombie is left until the session goes.
    let until = Instant::now() + DEADLINE;
    loop {
        let out = std::process::Command::new("/bin/ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        let stat = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if stat.is_empty() || stat.starts_with('Z') {
            break;
        }
        assert!(Instant::now() < until, "program still running: {stat}");
        std::thread::sleep(Duration::from_millis(20));
    }

    other.type_text("echo still-alive\n");
    other.wait_for("the other session", |d| has_line(d, "still-alive"));
}

// --- Full-screen programs ---------------------------------------------------

#[test]
fn vim_insert_and_quit() {
    if !have("vim") {
        eprintln!("skipped: no vim");
        return;
    }
    let h = Harness::shell("vim");
    h.type_text("vim -u NONE -N\n");
    h.wait_term("vim on the alt screen", |t| t.alt_screen_active().then_some(()));
    h.type_text("ihello");
    h.key(ESCAPE);
    h.wait_term("hello typed", |t| {
        let d = t.dump();
        (d.starts_with("hello|") && t.cursor().row() == 0).then_some(())
    });
    h.type_text(":q!\n");
    let d = h.wait_term("vim to quit", |t| {
        (!t.alt_screen_active() && prompts(&t.dump()) == 2).then(|| t.dump())
    });
    assert!(d.contains("READY> vim -u NONE -N"), "{d}");
}

#[test]
fn less_scrolls_with_arrow_keys() {
    let h = Harness::start("less", &[], |home| {
        let text: String = (1..=200).map(|i| format!("line {i}\n")).collect();
        std::fs::write(home.join("file.txt"), text).unwrap();
    });
    h.wait_for("prompt", |d| prompts(d) == 1);
    h.type_text("less file.txt\n");
    let top = |n: usize| move |d: &str| d.starts_with(&format!("line {n}|"));
    h.wait_for("less showing the file", top(1));
    for _ in 0..3 {
        h.key(DOWN);
    }
    h.wait_for("three lines down", top(4));
    h.key(UP);
    h.wait_for("one line up", top(3));
    // What the view sends for the scroll wheel on the alt screen.
    let app_cursor = h.modes().app_cursor;
    let wheel_down = keys::encode_key(keys::Key::Down, keys::Mods::default(), app_cursor);
    for _ in 0..5 {
        h.session().send(wheel_down.clone());
    }
    h.wait_for("wheel down", top(8));
    h.type_text("q");
    h.wait_term("less to quit", |t| {
        (!t.alt_screen_active() && prompts(&t.dump()) == 2).then_some(())
    });
}

const CURSES: &str = r#"
import curses

def main(scr):
    curses.start_color()
    curses.use_default_colors()
    curses.init_pair(1, curses.COLOR_RED, -1)
    scr.addstr(0, 0, "ready %d %d" % (curses.LINES, curses.COLS), curses.color_pair(1))
    scr.refresh()
    while True:
        k = scr.getch()
        if k == curses.KEY_RESIZE:
            curses.update_lines_cols()
            scr.erase()
            scr.addstr(0, 0, "size %d %d" % (curses.LINES, curses.COLS), curses.color_pair(1))
            scr.refresh()
        elif k == ord("q"):
            break

curses.wrapper(main)
print("curses done")
"#;

#[test]
fn python_curses_colours_and_resize() {
    if !python_with_curses() {
        eprintln!("skipped: no python3 with curses");
        return;
    }
    let h = Harness::start("curses", &[], |home| std::fs::write(home.join("c.py"), CURSES).unwrap());
    h.wait_for("prompt", |d| prompts(d) == 1);
    h.type_text("python3 -I c.py\n");
    let fg = h.wait_term("curses screen", |t| {
        (t.alt_screen_active() && t.dump().starts_with("ready 24 80|")).then(|| t.visible_row(0).cell(0).style().fg.kind())
    });
    assert_eq!(fg, ColorKind::Indexed(1), "red text");
    h.resize(100, 30);
    h.wait_for("LINES/COLS after SIGWINCH", |d| d.starts_with("size 30 100|"));
    h.type_text("q");
    h.wait_term("curses to end", |t| {
        let d = t.dump();
        (!t.alt_screen_active() && has_line(&d, "curses done") && prompts(&d) == 2).then_some(())
    });
}

const QUERIES: &str = r#"
import os, select, sys, termios, tty

fd = sys.stdin.fileno()
old = termios.tcgetattr(fd)
tty.setraw(fd)

def ask(query, done):
    os.write(1, query)
    got = b""
    while not done(got):
        ready, _, _ = select.select([fd], [], [], 5)
        if not ready:
            return got + b"<timeout>"
        got += os.read(fd, 1)
    return got

try:
    answers = [
        ("DA", ask(b"\x1b[c", lambda b: b.endswith(b"c"))),
        ("CPR", ask(b"\x1b[3;5H\x1b[6n", lambda b: b.endswith(b"R"))),
        ("DECRQM", ask(b"\x1b[?2004$p", lambda b: b.endswith(b"y"))),
        ("OSC11", ask(b"\x1b]11;?\x07", lambda b: b.endswith(b"\x07") or b.endswith(b"\x1b\\"))),
    ]
finally:
    termios.tcsetattr(fd, termios.TCSADRAIN, old)
os.write(1, b"\x1b[H\x1b[2J")
for name, value in answers:
    print(name + "=" + value.decode("latin-1").replace("\x1b", "ESC"))
"#;

#[test]
fn queries_round_trip_through_the_pty() {
    if !python_with_curses() {
        eprintln!("skipped: no python3");
        return;
    }
    let mut h = Harness::start("queries", &["python3", "-I", "q.py"], |home| {
        std::fs::write(home.join("q.py"), QUERIES).unwrap()
    });
    let d = h.wait_for("the replies", |d| d.contains("OSC11="));
    let lines: Vec<&str> = d.lines().take(4).collect();
    assert_eq!(
        lines,
        [
            "DA=ESC[?62;22c|",
            "CPR=ESC[3;5R|",
            "DECRQM=ESC[?2004;2$y|",
            "OSC11=ESC]11;rgb:1616/1818/1c1cESC\\|"
        ],
        "{d}"
    );
    h.wait_exit();
}

#[test]
fn bracketed_paste_reaches_the_program() {
    let h = Harness::shell("paste");
    h.type_text("printf '\\033[?2004h'; cat -v\n");
    h.wait_term("bracketed paste on", |t| t.modes().bracketed_paste.then_some(()));
    let bracketed = h.session().term().lock().unwrap().state.modes().bracketed_paste;
    h.session().send(keys::paste_bytes("one\ntwo\x1b[201~three", bracketed));
    h.type_text("\n");
    // Each line twice: the terminal's echo, then cat's output (so cat is
    // running and Ctrl+C reaches it).
    let twice = |d: &str, line: &str| d.lines().filter(|l| l.trim_end_matches('|') == line).count() == 2;
    h.wait_for("the pasted text", |d| twice(d, "^[[200~one") && twice(d, "two[201~three^[[201~"));
    h.ctrl('c');
    h.wait_for("prompt", |d| prompts(d) == 2);
}

#[test]
fn resize_while_a_program_runs() {
    let h = Harness::shell("stty");
    h.type_text("stty size\n");
    h.wait_for("24 80", |d| has_line(d, "24 80"));
    h.resize(100, 30);
    h.type_text("stty size\n");
    let d = h.wait_for("30 100", |d| has_line(d, "30 100"));
    assert_eq!(d.lines().count(), 31, "30 rows and the status line");
    h.resize(40, 10);
    h.type_text("clear; stty size\n");
    h.wait_for("10 40", |d| has_line(d, "10 40"));
}

// --- Reader and draw-loop concurrency --------------------------------------

/// A stand-in for the view's display link: ticks every 8 ms while running,
/// asks `frame_policy::decide`, "draws" by taking a screen dump, pauses after
/// two idle ticks and is resumed only by a wake-up.
struct FakeLoop {
    stop: Arc<AtomicBool>,
    draws: Arc<AtomicU64>,
    last_drawn: Arc<Mutex<String>>,
    paused: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

const TICK: Duration = Duration::from_millis(8);

impl FakeLoop {
    fn start(session: Arc<Session>, signal: Arc<Signal>) -> FakeLoop {
        let stop = Arc::new(AtomicBool::new(false));
        let draws = Arc::new(AtomicU64::new(0));
        let last_drawn = Arc::new(Mutex::new(String::new()));
        let paused = Arc::new(AtomicBool::new(false));
        let (stop2, draws2, last2, paused2) = (stop.clone(), draws.clone(), last_drawn.clone(), paused.clone());
        let thread = std::thread::spawn(move || {
            let mut idle_ticks = 0u8;
            let mut seen_wakes = signal.wakes();
            let mut sync_since: Option<Instant> = None;
            let long_ago = Duration::from_secs(10);
            while !stop2.load(Ordering::Acquire) {
                if paused2.load(Ordering::Acquire) {
                    // Paused: only a wake-up (or stopping) resumes drawing.
                    let mut ev = signal.events.lock().unwrap();
                    while ev.wakes == seen_wakes && !stop2.load(Ordering::Acquire) {
                        ev = signal.cv.wait_timeout(ev, Duration::from_millis(50)).unwrap().0;
                    }
                    seen_wakes = ev.wakes;
                    drop(ev);
                    idle_ticks = 0;
                    paused2.store(false, Ordering::Release);
                    // Draw at once, as the view does after a pause.
                } else {
                    std::thread::sleep(TICK);
                    seen_wakes = signal.wakes();
                }
                let dirty = session.take_dirty();
                let t = session.term().lock().unwrap();
                let synchronized = t.state.modes().synchronized;
                let elapsed = if synchronized {
                    sync_since.get_or_insert_with(Instant::now).elapsed()
                } else {
                    sync_since = None;
                    Duration::ZERO
                };
                match frame_policy::decide(dirty, true, synchronized, elapsed, idle_ticks, false, long_ago, true) {
                    Decision::Draw { .. } => {
                        idle_ticks = 0;
                        *last2.lock().unwrap() = t.state.dump();
                        draws2.fetch_add(1, Ordering::AcqRel);
                    }
                    Decision::Skip => idle_ticks = idle_ticks.saturating_add(1),
                    Decision::Pause { .. } => {
                        idle_ticks = idle_ticks.saturating_add(1);
                        paused2.store(true, Ordering::Release);
                    }
                    Decision::Hold => session.mark_dirty(),
                    Decision::Hide { .. } => unreachable!("always visible"),
                }
            }
        });
        FakeLoop {
            stop,
            draws,
            last_drawn,
            paused,
            thread: Some(thread),
        }
    }

    /// Wait until the loop is idle and its last frame shows `screen()`. A wake
    /// may be in flight when the program exits, so "paused" alone isn't
    /// enough; a lost wake-up never catches up and times out here.
    fn wait_settled(&self, within: Duration, screen: impl Fn() -> String) -> bool {
        let until = Instant::now() + within;
        loop {
            if self.paused.load(Ordering::Acquire) && *self.last_drawn.lock().unwrap() == screen() {
                return true;
            }
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(TICK);
        }
    }

    /// Wait until the loop has paused (nothing left to draw).
    #[allow(dead_code)]
    fn wait_paused(&self, within: Duration) -> bool {
        let until = Instant::now() + within;
        while !self.paused.load(Ordering::Acquire) {
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(TICK);
        }
        true
    }

    fn stop(mut self) -> u64 {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
        self.draws.load(Ordering::Acquire)
    }
}

/// Starts `script` under `sh -c` with a fake draw loop.
fn with_loop(name: &str, script: &str, sz: Size) -> (Arc<Session>, Arc<Signal>, FakeLoop, TempDir) {
    let home = TempDir::new(name);
    let cmd = command(&["/bin/sh", "-c", script], home.path());
    let signal = Arc::new(Signal::default());
    let term = Term::new(sz.cols as usize, sz.lines as usize, 1000);
    let session = Arc::new(Session::spawn(&cmd, sz, term, signal.hooks()).unwrap());
    let fake = FakeLoop::start(session.clone(), signal.clone());
    (session, signal, fake, home)
}

fn finish(session: Arc<Session>) {
    Arc::try_unwrap(session).ok().expect("loop still holds the session").shut_down();
}

/// Output arriving in bursts while the loop keeps pausing: every burst must
/// wake it, so the last frame drawn shows the final screen.
#[test]
fn no_lost_wake_up() {
    let script =
        "for i in 1 2 3 4 5 6 7 8; do printf 'burst %s ' $i; printf 'x%.0s' 1 2 3 4 5 6 7 8 9 10; echo; sleep 0.15; done; printf done";
    let (session, signal, fake, _home) = with_loop("wakeup", script, size(80, 24));
    assert!(signal.wait_exit(DEADLINE), "program didn't exit");
    let screen = || session.term().lock().unwrap().state.dump();
    let settled = fake.wait_settled(Duration::from_secs(5), screen);
    let last = fake.last_drawn.lock().unwrap().clone();
    let draws = fake.stop();
    let fin = screen();
    assert!(fin.contains("burst 8 xxxxxxxxxx|\ndone|"), "{fin}");
    assert!(settled, "the last frame drawn never became the final screen:\n{last}\n---\n{fin}");
    assert!(draws >= 8, "only {draws} draws for 8 bursts with pauses");
    assert!(signal.wakes() >= 8, "{} wakes", signal.wakes());
    finish(session);
}

/// Under a flood, the reader wakes the main thread only when no redraw is
/// pending, so there is at most one wake per frame drawn (plus the one in
/// flight).
#[test]
fn flood_wakes_at_most_once_per_frame() {
    let script = "yes 'flood flood flood flood flood flood flood flood flood flood' | head -n 300000";
    let (session, signal, fake, _home) = with_loop("flood", script, size(120, 40));
    assert!(signal.wait_exit(Duration::from_secs(60)), "program didn't exit");
    let screen = || session.term().lock().unwrap().state.dump();
    let settled = fake.wait_settled(Duration::from_secs(5), screen);
    let draws = fake.stop();
    let wakes = signal.wakes();
    assert!(wakes >= 1);
    assert!(wakes <= draws + 1, "{wakes} wakes for {draws} draws");
    assert!(settled, "the last frame drawn never became the final screen");
    finish(session);
}

/// Dropping the session hangs up even on a program that ignores SIGHUP and
/// a background job still holding the terminal: both threads end and the
/// terminal is freed within 3 s.
#[test]
fn drop_frees_everything() {
    let home = TempDir::new("drop");
    let script = "trap '' HUP; sleep 100 & echo started; while :; do sleep 1; done";
    let cmd = command(&["/bin/sh", "-c", script], home.path());
    let signal = Arc::new(Signal::default());
    let session = Session::spawn(&cmd, size(80, 24), Term::new(80, 24, 100), signal.hooks()).unwrap();
    let mut h = Harness {
        session: Some(session),
        signal: signal.clone(),
        _home: home,
    };
    h.wait_for("the program to start", |d| has_line(d, "started"));
    let session = h.session.take().unwrap();
    let weak = Arc::downgrade(session.term());
    let start = Instant::now();
    let threads = session.hang_up();
    assert_eq!(threads.len(), 2);
    let limit = Duration::from_secs(3);
    while !(threads.iter().all(|t| t.is_finished()) && weak.upgrade().is_none()) {
        assert!(
            start.elapsed() < limit,
            "after {:?}: threads finished {:?}, terminal freed {}",
            start.elapsed(),
            threads.iter().map(|t| t.is_finished()).collect::<Vec<_>>(),
            weak.upgrade().is_none()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(signal.exits(), 1);
}

const CPR_LOOP: &str = r#"
import os, re, select, sys, termios, tty

fd = sys.stdin.fileno()
old = termios.tcgetattr(fd)
tty.setraw(fd)
got = b""
try:
    for i in range(200):
        os.write(1, b"\x1b[6n")
        while got.count(b"R") <= i:
            ready, _, _ = select.select([fd], [], [], 5)
            if not ready:
                break
            got += os.read(fd, 4096)
finally:
    termios.tcsetattr(fd, termios.TCSADRAIN, old)
cpr = re.compile(rb"\x1b\[\d+;\d+R")
rest = cpr.sub(b"", got)
print("CPR %d BAD %d" % (len(cpr.findall(got)), len(rest) - rest.count(b"x")))
"#;

/// Query replies and typed input share the writer; a reply must never be
/// split by input.
#[test]
fn replies_stay_intact_while_typing() {
    if !python_with_curses() {
        eprintln!("skipped: no python3");
        return;
    }
    let mut h = Harness::start("cpr", &["python3", "-I", "cpr.py"], |home| {
        std::fs::write(home.join("cpr.py"), CPR_LOOP).unwrap()
    });
    let stop = AtomicBool::new(false);
    let (d, sent) = std::thread::scope(|scope| {
        // Typing on another thread (the main thread, in the app) while the
        // reader thread queues replies.
        let typist = scope.spawn(|| {
            let mut n = 0u64;
            while !stop.load(Ordering::Acquire) && h.signal.exits() == 0 && n < 20_000 {
                h.session().send(b"xxxxxxx".to_vec());
                n += 1;
                if n.is_multiple_of(64) {
                    std::thread::yield_now();
                }
            }
            n
        });
        let d = h.wait_for("the result", |d| d.contains("CPR "));
        stop.store(true, Ordering::Release);
        (d, typist.join().unwrap())
    });
    assert!(sent > 0);
    assert!(d.contains("CPR 200 BAD 0"), "{d}");
    h.wait_exit();
}

/// Resizing over and over while another thread floods output keeps the
/// terminal consistent; a panic on the reader thread (the debug-build
/// invariant check in `Term::advance`) fails the test.
#[test]
fn resize_under_flood() {
    let script = "yes 'wide 中文字符 and narrow text with tabs\tand more text to wrap around the screen'";
    let home = TempDir::new("resize-flood");
    let cmd = command(&["/bin/sh", "-c", script], home.path());
    let signal = Arc::new(Signal::default());
    let session = Session::spawn(&cmd, size(80, 24), Term::new(80, 24, 2000), signal.hooks()).unwrap();
    session.take_dirty();
    signal.wait_past(0, Instant::now() + DEADLINE);
    assert!(signal.wakes() > 0, "no output arrived");
    let mut seed = 12345u32;
    let mut next = |m: u32| {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        (seed >> 16) % m
    };
    let start = Instant::now();
    let mut i = 0;
    while i < 400 || start.elapsed() < Duration::from_secs(1) {
        i += 1;
        let (cols, lines) = (1 + next(200) as u16, 1 + next(60) as u16);
        session.resize(size(cols, lines)).unwrap();
        let t = session.term().lock().unwrap();
        if let Err(e) = t.state.check_invariants() {
            panic!("after resize {i} to {cols}x{lines}: {e}\n{}", t.state.dump());
        }
        assert_eq!((t.state.cols(), t.state.lines()), (cols.max(2) as usize, lines as usize));
    }
    assert_eq!(signal.exits(), 0, "the flood stopped early");
    let term = session.term().clone();
    session.shut_down();
    term.lock().unwrap().state.check_invariants().unwrap();
}

/// The test-only control OSC: each command reaches the hook after the
/// output before it has been parsed, and the terminal itself ignores it.
#[test]
fn control_commands_follow_the_output_before_them() {
    let home = TempDir::new("control");
    let script = r#"printf 'one\n\033]7770;dump\007two\n\033]7770;resize 3 4\033\\three\n'"#;
    let cmd = command(&["/bin/sh", "-c", script], home.path());
    let signal = Arc::new(Signal::default());
    let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let term: Arc<Mutex<Option<Arc<Mutex<Term>>>>> = Arc::default();
    let mut hooks = signal.hooks();
    let (seen2, term2) = (seen.clone(), term.clone());
    hooks.on_control = Some(Box::new(move |c| {
        // The session's terminal, once spawn has returned it.
        let t = loop {
            if let Some(t) = term2.lock().unwrap().clone() {
                break t;
            }
            std::thread::yield_now();
        };
        let screen = t.lock().unwrap().state.dump();
        seen2.lock().unwrap().push((c, screen));
    }));
    let session = Session::spawn(&cmd, size(20, 5), Term::new(20, 5, 100), hooks).unwrap();
    *term.lock().unwrap() = Some(session.term().clone());
    let mut h = Harness {
        session: Some(session),
        signal,
        _home: home,
    };
    h.wait_exit();
    let seen = seen.lock().unwrap();
    let cmds: Vec<&str> = seen.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(cmds, ["dump", "resize 3 4"]);
    assert!(seen[0].1.starts_with("one|\n|"), "{}", seen[0].1);
    assert!(seen[1].1.starts_with("one|\ntwo|\n|"), "{}", seen[1].1);
    let d = h.dump();
    assert!(d.starts_with("one|\ntwo|\nthree|\n|"), "the OSC must not show: {d}");
}
