//! Real PTY tests. Every child runs under `/bin/sh` (via `Command.shell`)
//! with HOME set to a fresh temp dir, so the user's shell and dotfiles don't
//! matter. Nothing sleeps to synchronise: output is read on a thread into a
//! channel and every wait has a deadline, so a broken case fails with the
//! output so far instead of hanging.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use term_pty::{Command, Pty, Size};

/// Upper bound for any single wait.
const DEADLINE: Duration = Duration::from_secs(15);

const SIZE: Size = Size {
    cols: 80,
    lines: 24,
    cell_width: 8,
    cell_height: 16,
};

/// A temp directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("decline-pty-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir.canonicalize().unwrap())
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn str(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A running child plus a reader thread feeding its output into a channel.
struct Session {
    pty: Option<Pty>,
    rx: Receiver<Vec<u8>>,
    out: Vec<u8>,
    exited: bool,
    _home: TempDir,
}

fn argv(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Spawns `argv` through `/bin/sh` with HOME set to a fresh temp dir;
/// `setup` can adjust the command first.
fn spawn(name: &str, args: &[&str], setup: impl FnOnce(&mut Command, &TempDir)) -> Session {
    let home = TempDir::new(name);
    let mut cmd = Command {
        argv: argv(args),
        env: vec![("HOME".into(), home.str().into())],
        shell: Some("/bin/sh".into()),
        ..Command::default()
    };
    setup(&mut cmd, &home);
    let pty = Pty::spawn(&cmd, SIZE).unwrap();
    let mut reader = pty.reader().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = reader.read_batch(&mut buf);
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                return;
            }
        }
    });
    Session {
        pty: Some(pty),
        rx,
        out: Vec::new(),
        exited: false,
        _home: home,
    }
}

impl Session {
    fn pty(&self) -> &Pty {
        self.pty.as_ref().unwrap()
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.out).into_owned()
    }

    /// Takes the next chunk of output; false once the reader has finished.
    fn recv(&mut self, deadline: Instant, what: &str) -> bool {
        let left = deadline.saturating_duration_since(Instant::now());
        match self.rx.recv_timeout(left) {
            Ok(bytes) => {
                self.out.extend_from_slice(&bytes);
                true
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.exited = true;
                false
            }
            Err(RecvTimeoutError::Timeout) => {
                panic!("timed out after {DEADLINE:?} waiting for {what}; output so far: {:?}", self.text())
            }
        }
    }

    /// Waits until the output so far satisfies `pred`.
    fn wait_for(&mut self, what: &str, pred: impl Fn(&[u8]) -> bool) {
        let deadline = Instant::now() + DEADLINE;
        while !pred(&self.out) {
            if self.exited || !self.recv(deadline, what) {
                panic!("program exited before {what}; output: {:?}", self.text());
            }
        }
    }

    fn wait_for_text(&mut self, needle: &str) {
        let n = needle.as_bytes().to_vec();
        self.wait_for(&format!("{needle:?}"), move |out| out.windows(n.len()).any(|w| w == n));
    }

    /// Waits until the program has exited, i.e. `read_batch` returned 0.
    fn wait_exit(&mut self) {
        let deadline = Instant::now() + DEADLINE;
        while !self.exited {
            self.recv(deadline, "exit");
        }
    }

    fn write(&self, bytes: &[u8]) {
        self.pty().writer().unwrap().write_all(bytes).unwrap();
    }
}

fn alive(pid: libc::pid_t) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

#[test]
fn output_then_exit() {
    let mut s = spawn("exit", &["sh", "-c", "printf hello; exit 3"], |_, _| {});
    s.wait_for_text("hello");
    s.wait_exit();
    assert!(s.text().ends_with("hello"), "{:?}", s.text());
}

#[test]
fn empty_argv_is_a_login_shell() {
    let mut s = spawn("login", &[], |_, _| {});
    // The echoed input contains "<%s>", not "<-sh>".
    s.write(b"printf '<%s>' \"$0\"; exit\n");
    s.wait_for_text("<-sh>");
    s.wait_exit();
}

#[test]
fn bare_program_found_via_path_gets_env() {
    let mut s = spawn("env", &["env"], |cmd, _| cmd.env.push(("DECLINE_TEST".into(), "a b".into())));
    s.wait_exit();
    let lines: Vec<String> = s.text().lines().map(|l| l.trim_end_matches('\r').to_owned()).collect();
    for want in [
        "DECLINE_TEST=a b",
        "TERM=xterm-256color",
        "COLORTERM=truecolor",
        "TERM_PROGRAM=decline",
    ] {
        assert!(lines.iter().any(|l| l == want), "missing {want:?} in {lines:?}");
    }
}

#[test]
fn argv_is_passed_unchanged() {
    let mut s = spawn("argv", &["printf", "[%s]", "a b", "$HOME", "*"], |_, _| {});
    s.wait_exit();
    assert!(s.text().contains("[a b][$HOME][*]"), "{:?}", s.text());
}

#[test]
fn resize_reaches_the_child() {
    let mut s = spawn("resize", &["sh", "-c", "read _; stty size"], |_, _| {});
    s.pty()
        .resize(Size {
            cols: 77,
            lines: 33,
            ..SIZE
        })
        .unwrap();
    s.write(b"\n");
    s.wait_exit();
    assert!(s.text().contains("33 77"), "{:?}", s.text());
}

#[test]
fn drop_kills_a_child_that_ignores_hup() {
    let mut s = spawn("reaper", &["sh", "-c", "trap '' HUP; echo ready; sleep 60"], |_, _| {});
    s.wait_for_text("ready");
    let pid = s.pty().pid();
    let start = Instant::now();
    drop(s.pty.take());
    // Polled with a deadline: the reaper gives SIGHUP a 2 s grace period,
    // then SIGKILLs. Gone before 1.5 s would mean SIGHUP alone killed it,
    // so the SIGKILL path wasn't exercised.
    while alive(pid) {
        assert!(start.elapsed() < Duration::from_secs(5), "child {pid} still alive after 5 s");
        std::thread::sleep(Duration::from_millis(10));
    }
    let gone = start.elapsed();
    assert!(gone >= Duration::from_millis(1500), "child gone after {gone:?}, before SIGKILL");
    // The `sleep` it started is killed too, so the PTY closes.
    s.wait_exit();
}

#[test]
fn write_one_megabyte() {
    let mut s = spawn(
        "write",
        &["sh", "-c", "stty raw -echo; echo ready; exec head -c 1048576 >/dev/null; echo done"],
        |_, _| {},
    );
    s.wait_for_text("ready");
    let mut writer = s.pty().writer().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(writer.write_all(&vec![b'x'; 1 << 20]));
    });
    s.wait_exit();
    let res = rx.recv_timeout(DEADLINE).unwrap_or_else(|e| panic!("write_all didn't return: {e}"));
    assert!(res.is_ok(), "{res:?}");
    assert!(!s.text().contains("done"), "exec didn't replace the shell: {:?}", s.text());
}

#[cfg(target_vendor = "apple")]
#[test]
fn foreground_cwd_is_canonical() {
    let mut s = spawn("fgcwd", &["sh", "-c", "echo ready; read _"], |cmd, _| cmd.cwd = Some("/tmp".into()));
    s.wait_for_text("ready");
    let want = std::fs::canonicalize("/tmp").unwrap();
    assert_eq!(s.pty().foreground_cwd().as_deref(), want.to_str());
    s.write(b"\n");
    s.wait_exit();
}

#[cfg(not(target_vendor = "apple"))]
#[test]
fn foreground_cwd_is_none_off_macos() {
    let mut s = spawn("fgcwd", &["sh", "-c", "echo ready; read _"], |_, _| {});
    s.wait_for_text("ready");
    assert_eq!(s.pty().foreground_cwd(), None);
    s.write(b"\n");
    s.wait_exit();
}

/// Runs `pwd -P` with the given cwd and returns what it printed.
fn pwd(name: &str, cwd: impl FnOnce(&TempDir) -> Option<String>) -> (String, PathBuf) {
    let mut home = PathBuf::new();
    let mut s = spawn(name, &["pwd", "-P"], |cmd, h| {
        cmd.cwd = cwd(h);
        home = h.path().to_owned();
    });
    s.wait_exit();
    (s.text().trim().to_owned(), home)
}

#[test]
fn cwd_is_honoured() {
    let (out, home) = pwd("cwd", |h| {
        let sub = h.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        Some(sub.to_str().unwrap().into())
    });
    assert_eq!(Path::new(&out), home.join("sub"));
}

#[test]
fn missing_cwd_falls_back_to_home() {
    let (out, home) = pwd("nocwd", |h| Some(h.path().join("does-not-exist").to_str().unwrap().into()));
    assert_eq!(Path::new(&out), home);
}

#[test]
fn no_cwd_starts_in_home() {
    let (out, home) = pwd("homecwd", |_| None);
    assert_eq!(Path::new(&out), home);
}
