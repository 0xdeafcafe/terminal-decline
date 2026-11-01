//! The PTY read on a thread of its own (`pty-drain`), apart from parsing.
//!
//! A macOS PTY holds about 2 KB and hands out about 1 KB per read, so a
//! program writing a flood (`cat` of a big file) waits on whoever reads
//! it. With one thread reading and parsing, the program waited while
//! each piece was parsed, while the terminal's lock was held for a
//! frame, and for the parse's own bookkeeping. Here the drain thread does
//! nothing but read, appending to a buffer; the reader thread takes all
//! that is waiting in one go and parses it under one lock. Reading and
//! parsing overlap, the parse sees bigger pieces, and the program only
//! waits once the parser is [`CAP`] behind.
//!
//! The extra hop costs a wake-up when the parser is waiting (a key's
//! echo), a few microseconds.

use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use term_pty::BatchReader;

/// How far the drain may get ahead of the parser before it stops reading,
/// so the program waits: a few milliseconds of parsing. More would only
/// cost memory, and output still to parse after Ctrl+C.
pub const CAP: usize = 1024 * 1024;

/// What a read can return at most: a macOS PTY gives ~1 KB.
const READ: usize = 16 * 1024;

/// A buffer kept after a flood is given back down to this.
const KEEP: usize = 64 * 1024;

#[derive(Default)]
struct State {
    buf: Vec<u8>,
    /// The program's output has ended (or the drain stopped).
    ended: bool,
    /// Nobody will take what is read: read and drop it, so the program
    /// can finish exiting.
    dropped: bool,
    parser_waiting: bool,
    drain_waiting: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    /// Bytes arrived, or the output ended.
    ready: Condvar,
    /// The parser took the bytes, or went away.
    room: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The parser's end: takes what the drain has read.
pub struct Taker {
    shared: Arc<Shared>,
}

impl Taker {
    /// Replace `into`'s contents with everything read since the last take,
    /// waiting while nothing is. False once the output has ended and all
    /// of it was taken.
    pub fn take(&self, into: &mut Vec<u8>) -> bool {
        into.clear();
        if into.capacity() > KEEP {
            into.shrink_to(KEEP);
        }
        let mut s = self.shared.lock();
        while s.buf.is_empty() && !s.ended {
            s.parser_waiting = true;
            s = self.shared.ready.wait(s).unwrap_or_else(PoisonError::into_inner);
            s.parser_waiting = false;
        }
        if s.buf.is_empty() {
            return false;
        }
        std::mem::swap(&mut s.buf, into);
        if s.drain_waiting {
            self.shared.room.notify_one();
        }
        true
    }
}

impl Drop for Taker {
    fn drop(&mut self) {
        let mut s = self.shared.lock();
        s.dropped = true;
        s.buf = Vec::new();
        self.shared.room.notify_one();
    }
}

/// Marks the output ended however the drain thread stops.
struct EndOnDrop(Arc<Shared>);

impl Drop for EndOnDrop {
    fn drop(&mut self) {
        self.0.lock().ended = true;
        self.0.ready.notify_one();
    }
}

/// Start the drain thread on `reader`.
pub fn start(mut reader: BatchReader) -> io::Result<(Taker, JoinHandle<()>)> {
    let shared = Arc::new(Shared::default());
    let end = EndOnDrop(shared.clone());
    let thread = std::thread::Builder::new().name("pty-drain".into()).spawn(move || {
        let shared = end.0.clone();
        let mut chunk = vec![0u8; READ];
        loop {
            let n = reader.read_batch(&mut chunk);
            if n == 0 {
                break;
            }
            let mut s = shared.lock();
            while s.buf.len() >= CAP && !s.dropped {
                s.drain_waiting = true;
                s = shared.room.wait(s).unwrap_or_else(PoisonError::into_inner);
                s.drain_waiting = false;
            }
            if s.dropped {
                continue;
            }
            s.buf.extend_from_slice(&chunk[..n]);
            if s.parser_waiting {
                shared.ready.notify_one();
            }
        }
        drop(end);
    })?;
    Ok((Taker { shared }, thread))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use term_pty::{Command, Pty, Size};

    fn size() -> Size {
        Size {
            cols: 80,
            lines: 24,
            cell_width: 8,
            cell_height: 16,
        }
    }

    fn sh(script: &str) -> Command {
        Command {
            argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
            direct: true,
            ..Command::default()
        }
    }

    /// Everything a program writes arrives, in order, in pieces bigger
    /// than one read once the parser falls behind, and then the end.
    #[test]
    fn all_of_a_flood_arrives_in_order() {
        let cmd = sh("i=0; while [ $i -lt 20000 ]; do echo line$i; i=$((i+1)); done");
        let pty = Pty::spawn(&cmd, size()).unwrap();
        let (taker, thread) = start(pty.reader().unwrap()).unwrap();
        let mut all = Vec::new();
        let mut piece = Vec::new();
        let mut biggest = 0;
        // Slow at first, so the drain gets ahead.
        std::thread::sleep(Duration::from_millis(300));
        while taker.take(&mut piece) {
            biggest = biggest.max(piece.len());
            all.extend_from_slice(&piece);
        }
        thread.join().unwrap();
        let text = String::from_utf8(all).unwrap();
        let lines: Vec<&str> = text.split("\r\n").filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 20000);
        assert!(lines.iter().enumerate().all(|(i, l)| *l == format!("line{i}")));
        assert!(biggest > 4096, "taken together: {biggest}");
        assert!(biggest <= CAP + READ);
    }

    /// With nobody taking, the program still finishes: the drain stops
    /// at the cap until the taker goes, then reads and drops the rest.
    #[test]
    fn a_dropped_taker_lets_the_program_finish() {
        let cmd = sh("head -c 5000000 /dev/zero");
        let pty = Pty::spawn(&cmd, size()).unwrap();
        let (taker, thread) = start(pty.reader().unwrap()).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(taker.shared.lock().buf.len() <= CAP + READ);
        drop(taker);
        thread.join().unwrap();
    }
}
