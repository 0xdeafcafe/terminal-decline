//! Where a `cat` flood's time goes, without a window: the PTY on its own,
//! the PTY with the parse the app's reader does, and the parse alone.
//!
//! ```text
//! cargo run --release -p term-pty --example flood -- FILE [--reps N] [--stty "speed 38400"]
//! ```
//!
//! For each of `memory` (the file parsed from memory in 1 KB pieces, as a
//! macOS PTY hands it out), `discard` (`cat FILE` on a PTY, read with
//! [`term_pty::BatchReader`] and dropped) and `parse` (the same, parsed
//! under a lock as the app's reader does, 100×30 with 10k lines of
//! scrollback), prints one line per rep: wall seconds, this process's CPU
//! seconds, the reads and how big they were. `pump` is `parse` as the app
//! does it now: reads on a `pty-read` thread ([`term_pty::Pump`]), parsed
//! on this one. `--stty` runs `stty ARGS` in the PTY before `cat`.

use std::sync::Mutex;
use std::time::Instant;

use term_core::Term;
use term_pty::{Command, Pty, Size};

fn cpu_s() -> f64 {
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
    let t = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 * 1e-6;
    t(u.ru_utime) + t(u.ru_stime)
}

fn term() -> Term {
    Term::new(100, 30, 10_000, term_core::ImagePool::process())
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut reps = 3;
    let mut stty = String::new();
    let mut modes = vec!["memory".to_string(), "discard".into(), "parse".into(), "pump".into()];
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--reps" => {
                reps = args[i + 1].parse().expect("--reps N");
                args.drain(i..i + 2);
            }
            "--stty" => {
                stty = args[i + 1].clone();
                args.drain(i..i + 2);
            }
            "--modes" => {
                modes = args[i + 1].split(',').map(str::to_string).collect();
                args.drain(i..i + 2);
            }
            _ => i += 1,
        }
    }
    let file = args
        .first()
        .expect("usage: flood FILE [--reps N] [--stty ARGS] [--modes memory,discard,parse,pump]");
    let data = std::fs::read(file).expect("read FILE");
    for mode in &modes {
        for rep in 0..reps {
            if mode == "memory" {
                let t = Mutex::new(term());
                let (w0, c0) = (Instant::now(), cpu_s());
                for piece in data.chunks(1024) {
                    let mut t = t.lock().unwrap();
                    t.advance(piece);
                    std::hint::black_box(t.state.take_replies());
                }
                println!(
                    "mode=memory rep={rep} wall_s={:.3} cpu_s={:.3} bytes={}",
                    w0.elapsed().as_secs_f64(),
                    cpu_s() - c0,
                    data.len()
                );
                continue;
            }
            let parse = mode == "parse";
            let script = if stty.is_empty() {
                format!("exec cat '{file}'")
            } else {
                format!("stty {stty} >/dev/null 2>&1; exec cat '{file}'")
            };
            let cmd = Command {
                argv: vec!["/bin/sh".into(), "-c".into(), script],
                cwd: None,
                env: vec![],
                shell: None,
                direct: true,
                integration: None,
                reports: None,
            };
            let size = Size {
                cols: 100,
                lines: 30,
                cell_width: 8,
                cell_height: 16,
            };
            let t = Mutex::new(term());
            let (w0, c0) = (Instant::now(), cpu_s());
            let pty = Pty::spawn(&cmd, size).expect("spawn");
            let mut reader = pty.reader().expect("reader");
            let mut buf = vec![0u8; 64 * 1024];
            if mode == "pump" {
                // The app's way since rush/perf-cat: a pty-read thread, and
                // this one parses in pieces of at most 16 KB.
                let mut pump = term_pty::Pump::start(reader).expect("pump");
                let (mut total, mut batches) = (0usize, 0usize);
                let mut batch = Vec::new();
                while pump.next(&mut batch) {
                    total += batch.len();
                    batches += 1;
                    for piece in batch.chunks(16 * 1024) {
                        let mut t = t.lock().unwrap();
                        t.advance(piece);
                        std::hint::black_box(t.state.take_replies());
                    }
                }
                println!(
                    "mode=pump rep={rep} stty={stty:?} wall_s={:.3} cpu_s={:.3} bytes={total} batches={batches} avg_batch={}",
                    w0.elapsed().as_secs_f64(),
                    cpu_s() - c0,
                    total / batches.max(1),
                );
                drop(pty);
                continue;
            }
            let (mut total, mut reads) = (0usize, 0usize);
            // Reads of < 256, < 1024, 1024–2047, >= 2048 bytes.
            let mut sizes = [0usize; 4];
            let mut biggest = 0;
            loop {
                let n = reader.read_batch(&mut buf);
                if n == 0 {
                    break;
                }
                total += n;
                reads += 1;
                biggest = biggest.max(n);
                sizes[match n {
                    0..256 => 0,
                    256..1024 => 1,
                    1024..2048 => 2,
                    _ => 3,
                }] += 1;
                if parse {
                    let mut t = t.lock().unwrap();
                    t.advance(&buf[..n]);
                    std::hint::black_box(t.state.take_replies());
                }
            }
            println!(
                "mode={mode} rep={rep} stty={stty:?} wall_s={:.3} cpu_s={:.3} bytes={total} reads={reads} \
                 avg_read={} biggest={biggest} sizes(<256,<1k,1-2k,>=2k)={sizes:?}",
                w0.elapsed().as_secs_f64(),
                cpu_s() - c0,
                total / reads.max(1),
            );
            drop(pty);
        }
    }
}
