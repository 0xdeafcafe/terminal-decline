//! A headless terminal for conformance suites: runs a program on a PTY,
//! feeds its output to a `term_core::Term` and sends the terminal's replies
//! back, with no window. `scripts/conformance/run.py` drives it to run
//! vttest and esctest2 (see `docs/conformance.md`).
//!
//! ```text
//! cargo run -p term-pty --example headless -- [--cols N] [--lines N] -- program args...
//! ```
//!
//! Commands arrive on stdin, one per line; answers go to stdout:
//!
//! - `send <hex>`: write the bytes to the program.
//! - `enter`: press Return (CR, or CR LF in newline mode, as the app does).
//! - `idle <quiet_ms> <max_ms>`: wait until the program has been quiet for
//!   `quiet_ms` (or `max_ms` passed); answers `ok`.
//! - `screen`: answers `screen <lines>`, then each visible row (trailing
//!   blanks dropped), then `cursor <row> <col>` (0-based).
//! - `wait <max_ms>`: wait for the program to exit; answers `exit <code>`
//!   or `timeout`.
//!
//! End of input kills the program.
//!
//! One addition to what the app does: the harness answers DECRQCRA
//! (`CSI Pi;Pg;Pt;Pl;Pb;Pr * y`, checksum of a rectangle) itself, from the
//! terminal's screen, as xterm does after patch #334 (the sum of the
//! characters, blanks counted as spaces). esctest2 reads the screen back
//! through it; term-core doesn't implement the query.

use std::io::{BufRead, Write};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use term_core::Term;
use term_pty::{Command, Pty, Size, Writer};

struct Shared {
    term: Mutex<Term>,
    /// When output last arrived, and whether the program has gone.
    activity: Mutex<(Instant, bool)>,
    cv: Condvar,
}

/// Finds DECRQCRA in the output, across reads.
#[derive(Default)]
struct Scanner {
    state: ScanState,
    params: Vec<u8>,
    inter: Vec<u8>,
}

#[derive(Default, PartialEq)]
enum ScanState {
    #[default]
    Ground,
    Esc,
    Csi,
}

impl Scanner {
    /// Feeds one byte; returns the parameters of a complete DECRQCRA.
    fn feed(&mut self, b: u8) -> Option<Vec<u32>> {
        match self.state {
            ScanState::Ground => {
                if b == 0x1b {
                    self.state = ScanState::Esc;
                }
            }
            ScanState::Esc => {
                self.state = if b == b'[' { ScanState::Csi } else { ScanState::Ground };
                if b == 0x1b {
                    self.state = ScanState::Esc;
                }
                self.params.clear();
                self.inter.clear();
            }
            ScanState::Csi => match b {
                0x30..=0x3f => self.params.push(b),
                0x20..=0x2f => self.inter.push(b),
                0x40..=0x7e => {
                    self.state = ScanState::Ground;
                    if b == b'y' && self.inter == b"*" {
                        let text = String::from_utf8_lossy(&self.params).into_owned();
                        return Some(text.split(';').map(|p| p.parse().unwrap_or(0)).collect());
                    }
                }
                0x1b => self.state = ScanState::Esc,
                _ => self.state = ScanState::Ground,
            },
        }
        None
    }
}

/// The DECRQCRA reply for `params` (Pid; Pp; Pt; Pl; Pb; Pr), 1-based and
/// inclusive; missing edges are the screen's.
fn checksum_reply(term: &Term, params: &[u32]) -> Vec<u8> {
    let state = &term.state;
    let (lines, cols) = (state.lines(), state.cols());
    let get = |i: usize, default: usize| params.get(i).map(|&v| v as usize).filter(|&v| v != 0).unwrap_or(default);
    let id = params.first().copied().unwrap_or(0);
    let top = get(2, 1).min(lines);
    let left = get(3, 1).min(cols);
    let bottom = get(4, lines).min(lines);
    let right = get(5, cols).min(cols);
    let mut sum: u32 = 0;
    for line in top - 1..bottom {
        let row = state.visible_row(line);
        for col in left - 1..right {
            if col >= row.len() {
                sum = sum.wrapping_add(' ' as u32);
                continue;
            }
            let cell = row.cell(col);
            if cell.is_spacer() {
                continue;
            }
            let ch = if cell.is_blank() { ' ' } else { cell.ch() };
            sum = sum.wrapping_add(ch as u32);
        }
    }
    format!("\x1bP{id}!~{:04X}\x1b\\", sum & 0xffff).into_bytes()
}

fn screen(term: &Term) -> String {
    let state = &term.state;
    let mut out = format!("screen {}\n", state.lines());
    for line in 0..state.lines() {
        let row = state.visible_row(line);
        let text: String = row
            .cells()
            .filter(|c| !c.is_spacer())
            .map(|c| if c.is_blank() { ' ' } else { c.ch() })
            .collect();
        out.push_str(text.trim_end());
        out.push('\n');
    }
    let cursor = state.cursor();
    out.push_str(&format!("cursor {} {}\n", cursor.row(), cursor.col()));
    out
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .filter_map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok())
        .collect()
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let (mut cols, mut lines) = (80u16, 24u16);
    while let Some(flag) = args.first().cloned() {
        match flag.as_str() {
            "--cols" | "--lines" => {
                let v: u16 = args.get(1).and_then(|v| v.parse().ok()).expect("a number");
                if flag == "--cols" {
                    cols = v
                } else {
                    lines = v
                }
                args.drain(..2);
            }
            "--" => {
                args.remove(0);
                break;
            }
            _ => break,
        }
    }
    assert!(!args.is_empty(), "usage: headless [--cols N] [--lines N] -- program args...");

    let size = Size {
        cols,
        lines,
        cell_width: 8,
        cell_height: 16,
    };
    let cmd = Command {
        argv: args,
        cwd: std::env::current_dir().ok().map(|d| d.to_string_lossy().into_owned()),
        env: vec![("ENV".into(), String::new())],
        shell: Some("/bin/sh".into()),
        direct: false,
        integration: None,
    };
    let pty = Pty::spawn(&cmd, size).expect("spawn");
    let child = pty.child();
    let mut term = Term::new(cols as usize, lines as usize, 1000, term_core::ImagePool::process());
    term.state.set_cell_pixels((8, 16));
    let shared = Arc::new(Shared {
        term: Mutex::new(term),
        activity: Mutex::new((Instant::now(), false)),
        cv: Condvar::new(),
    });
    let writer: Arc<Mutex<Writer>> = Arc::new(Mutex::new(pty.writer().expect("writer")));

    let mut reader = pty.reader().expect("reader");
    let (sh, w) = (shared.clone(), writer.clone());
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        let mut scanner = Scanner::default();
        loop {
            let n = reader.read_batch(&mut buf);
            if n == 0 {
                break;
            }
            let mut replies = Vec::new();
            {
                let mut term = sh.term.lock().unwrap();
                let mut start = 0;
                for i in 0..n {
                    if let Some(params) = scanner.feed(buf[i]) {
                        term.advance(&buf[start..=i]);
                        start = i + 1;
                        replies.extend(term.state.take_replies());
                        replies.extend(checksum_reply(&term, &params));
                    }
                }
                term.advance(&buf[start..n]);
                replies.extend(term.state.take_replies());
            }
            if !replies.is_empty() {
                let _ = w.lock().unwrap().write_all(&replies);
            }
            sh.activity.lock().unwrap().0 = Instant::now();
            sh.cv.notify_all();
        }
        sh.activity.lock().unwrap().1 = true;
        sh.cv.notify_all();
    });

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let mut words = line.split_whitespace();
        let cmd = words.next().unwrap_or("");
        let mut num = || words.next().and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        let answer = match cmd {
            "send" => {
                let bytes = hex(line.trim_start_matches("send").trim());
                let _ = writer.lock().unwrap().write_all(&bytes);
                shared.activity.lock().unwrap().0 = Instant::now();
                "ok\n".to_string()
            }
            "enter" => {
                // The Return key, as the app sends it (CR LF in LNM).
                let lnm = shared.term.lock().unwrap().state.modes().linefeed_newline;
                let _ = writer.lock().unwrap().write_all(if lnm { b"\r\n" } else { b"\r" });
                shared.activity.lock().unwrap().0 = Instant::now();
                "ok\n".to_string()
            }
            "idle" => {
                let (quiet, max) = (Duration::from_millis(num()), Duration::from_millis(num()));
                let until = Instant::now() + max;
                let mut act = shared.activity.lock().unwrap();
                loop {
                    let now = Instant::now();
                    let quiet_at = act.0 + quiet;
                    if act.1 || now >= quiet_at || now >= until {
                        break;
                    }
                    act = shared.cv.wait_timeout(act, quiet_at.min(until) - now).unwrap().0;
                }
                "ok\n".to_string()
            }
            "screen" => screen(&shared.term.lock().unwrap()),
            "wait" => {
                let until = Instant::now() + Duration::from_millis(num());
                let mut status = None;
                while Instant::now() < until {
                    if let Some(s) = child.try_wait() {
                        status = Some(s);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                match status {
                    Some(s) => format!("exit {}\n", s.code().unwrap_or(-1)),
                    None => "timeout\n".to_string(),
                }
            }
            "" => continue,
            other => format!("error unknown command {other}\n"),
        };
        if out.write_all(answer.as_bytes()).and_then(|_| out.flush()).is_err() {
            break;
        }
    }
    child.signal(libc::SIGKILL, true);
}
