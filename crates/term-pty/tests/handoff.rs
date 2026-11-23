//! A program on a PTY handed from one `Pty` to another, as a restart
//! into a new build does between two processes (docs/design/handoff.md):
//! it keeps running, and its output and input flow through the new one.

use std::time::{Duration, Instant};

use term_pty::handoff::{self, Item};
use term_pty::{Batch, BatchReader, Command, Handed, Pty, Size};

const DEADLINE: Duration = Duration::from_secs(15);

const SIZE: Size = Size {
    cols: 80,
    lines: 24,
    cell_width: 8,
    cell_height: 16,
};

/// `/bin/sh -c script` on a PTY.
fn spawn(script: &str) -> Pty {
    let cmd = Command {
        argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
        shell: Some("/bin/sh".into()),
        env: vec![("HOME".into(), std::env::temp_dir().display().to_string())],
        ..Command::default()
    };
    Pty::spawn(&cmd, SIZE).unwrap()
}

/// Read until `out` contains `needle`; panics with what came if the
/// program ends or the readers stop first.
fn read_until(reader: &mut BatchReader, out: &mut String, needle: &str) {
    let mut buf = vec![0u8; 64 * 1024];
    while !out.contains(needle) {
        match reader.read(&mut buf) {
            Batch::Bytes(n) => out.push_str(&String::from_utf8_lossy(&buf[..n])),
            end => panic!("{end:?} before {needle:?}; output: {out:?}"),
        }
    }
}

fn alive(pid: libc::pid_t) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

fn wait_gone(pid: libc::pid_t) {
    let deadline = Instant::now() + DEADLINE;
    while alive(pid) {
        assert!(Instant::now() < deadline, "{pid} is still running");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The readers stop at once though the program prints nothing, and say
/// stopped, not ended.
#[test]
fn readers_stop_while_waiting_and_say_so() {
    let pty = spawn("echo ready; exec cat");
    let mut reader = pty.reader().unwrap();
    let mut out = String::new();
    read_until(&mut reader, &mut out, "ready");
    let waiting = std::thread::spawn(move || {
        let mut buf = [0u8; 1024];
        reader.read(&mut buf)
    });
    // Let it reach its wait; either way the answer is the same.
    std::thread::sleep(Duration::from_millis(50));
    let started = Instant::now();
    pty.stop_readers();
    assert_eq!(waiting.join().unwrap(), Batch::Stopped);
    assert!(started.elapsed() < Duration::from_secs(5));
    // A reader made after stays stopped, and the old call says 0.
    let mut later = pty.reader().unwrap();
    assert_eq!(later.read_batch(&mut [0u8; 16]), 0);
    assert!(alive(pty.pid()));
}

/// Handed over through the socket and adopted: the first `Pty` is
/// dropped and the program carries on, reading what the new one writes
/// and writing where the new one reads; nothing printed in between is
/// lost. When the adopted one is dropped the program gets its hang-up.
#[test]
fn a_program_carries_on_under_the_pty_that_adopts_it() {
    // Echoes each line with a counter, so a restarted program would
    // show: the counter would start again.
    let mut old = spawn("n=0; echo ready; while read line; do n=$((n+1)); echo \"$n:$line\"; done; echo done");
    let pid = old.pid();
    let tty = old.tty_name().map(str::to_string);
    let mut reader = old.reader().unwrap();
    let mut out = String::new();
    read_until(&mut reader, &mut out, "ready");
    old.writer().unwrap().write_all(b"one\n").unwrap();
    read_until(&mut reader, &mut out, "1:one");

    old.stop_readers();
    assert_eq!(reader.read(&mut [0u8; 1024]), Batch::Stopped);
    // Typed after the readers stopped: its echo waits in the PTY.
    old.writer().unwrap().write_all(b"two\n").unwrap();
    let handed = old.hand_over().unwrap();

    let path = std::env::temp_dir().join(format!("td-ho-pty-{}.sock", std::process::id()));
    let listener = handoff::listen(&path).unwrap();
    let items = vec![Item {
        fd: handed.master,
        header: format!("{}", handed.pid).into_bytes(),
        body: Vec::new(),
    }];
    let taker = {
        let path = path.clone();
        std::thread::spawn(move || handoff::take(&path, DEADLINE).unwrap())
    };
    handoff::serve(&listener, &items, DEADLINE).unwrap();
    let mut taken = taker.join().unwrap();
    let _ = std::fs::remove_file(&path);
    drop(items);
    // The old app goes: no hang-up.
    drop(old);
    std::thread::sleep(Duration::from_millis(200));
    assert!(alive(pid), "dropping a handed-over Pty ended its program");

    let item = taken.pop().unwrap();
    assert_eq!(item.header, format!("{pid}").into_bytes());
    let new = Pty::adopt(Handed {
        master: item.fd,
        pid,
        tty_name: tty.clone(),
    })
    .unwrap();
    assert_eq!((new.pid(), new.tty_name()), (pid, tty.as_deref()));
    assert!(new.child().try_wait().is_none());
    let mut reader = new.reader().unwrap();
    let mut out = String::new();
    read_until(&mut reader, &mut out, "2:two");
    new.writer().unwrap().write_all(b"three\n").unwrap();
    read_until(&mut reader, &mut out, "3:three");
    new.resize(Size { cols: 40, ..SIZE }).unwrap();
    assert!(new.shell_in_foreground());

    // The pane closes: the program gets SIGHUP, as from the Pty that
    // started it.
    drop(new);
    wait_gone(pid);
}

/// An adopted program that exits by itself ends the reader, and counts
/// as exited.
#[test]
fn an_adopted_program_that_exits_is_seen_to() {
    let mut old = spawn("echo ready; read line; echo bye");
    let pid = old.pid();
    let mut reader = old.reader().unwrap();
    let mut out = String::new();
    read_until(&mut reader, &mut out, "ready");
    old.stop_readers();
    let handed = old.hand_over().unwrap();
    drop(old);
    let new = Pty::adopt(handed).unwrap();
    let mut reader = new.reader().unwrap();
    new.writer().unwrap().write_all(b"go\n").unwrap();
    let mut out = String::new();
    read_until(&mut reader, &mut out, "bye");
    let mut buf = [0u8; 1024];
    loop {
        match reader.read(&mut buf) {
            Batch::Bytes(_) => {}
            end => {
                assert_eq!(end, Batch::Ended);
                break;
            }
        }
    }
    let deadline = Instant::now() + DEADLINE;
    while new.child().try_wait().is_none() {
        assert!(Instant::now() < deadline, "{pid} never counted as exited");
        std::thread::sleep(Duration::from_millis(10));
    }
}
