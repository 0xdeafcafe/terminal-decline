//! Reading the PTY on a thread of its own, so a flood's reads overlap its
//! parse.
//!
//! A macOS PTY hands out at most 1 KB per read and each read costs a few
//! microseconds in the kernel, about as long as parsing what it returned.
//! Read and parsed on one thread, the two add up; here a `pty-read` thread
//! reads into a buffer while the caller parses the last batch, so a flood
//! goes as fast as the slower of the two. The buffer is bounded
//! ([`PUMP_CAP`]): when the parse falls behind, the reads stop and the PTY
//! slows the program down, as before.
//!
//! The reader doesn't wake the parser for every read in a flood: it wakes
//! it when the buffer has [`PUMP_WAKE`] bytes, when a read came back short
//! (typing, a prompt) or when the PTY is empty, so a lone keystroke's echo
//! is handed over at once.

use std::io;
use std::os::fd::AsRawFd;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

use crate::{BatchReader, FLOOD_SPIN, PTY_READ_MAX, wait_fd};

/// At most this much output read but not yet taken by the parser.
pub const PUMP_CAP: usize = 64 * 1024;

/// In a flood, wake a waiting parser once this much is buffered.
const PUMP_WAKE: usize = 8 * 1024;

#[derive(Default)]
struct State {
    buf: Vec<u8>,
    /// The program has exited and everything it wrote is in `buf`.
    eof: bool,
    /// The parser has gone ([`Pump`] dropped): stop reading.
    closed: bool,
    /// The parser is waiting on `data`.
    parser_waiting: bool,
    /// The reader is waiting on `room`.
    reader_waiting: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    /// Signalled when there is output to take, or at the end.
    data: Condvar,
    /// Signalled when the parser took the buffer, or went away.
    room: Condvar,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The parser's end: [`Pump::next`] takes what the `pty-read` thread has
/// read so far.
pub struct Pump {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Pump {
    /// Start a `pty-read` thread reading from `reader`.
    pub fn start(reader: BatchReader) -> io::Result<Pump> {
        let shared = Arc::new(Shared::default());
        let s = shared.clone();
        let thread = std::thread::Builder::new()
            .name("pty-read".into())
            .spawn(move || read_loop(reader, &s))?;
        Ok(Pump {
            shared,
            thread: Some(thread),
        })
    }

    /// Replace `out`'s contents with all the output read since the last
    /// call, waiting while there is none. Returns false (with `out` empty)
    /// when the program has exited and everything has been taken.
    pub fn next(&mut self, out: &mut Vec<u8>) -> bool {
        out.clear();
        let mut st = self.shared.lock();
        loop {
            if !st.buf.is_empty() {
                std::mem::swap(&mut st.buf, out);
                let full = st.reader_waiting;
                drop(st);
                if full {
                    self.shared.room.notify_one();
                }
                return true;
            }
            if st.eof {
                return false;
            }
            st.parser_waiting = true;
            st = self.shared.data.wait(st).unwrap_or_else(PoisonError::into_inner);
            st.parser_waiting = false;
        }
    }
}

impl Drop for Pump {
    fn drop(&mut self) {
        let mut st = self.shared.lock();
        st.closed = true;
        let ended = st.eof;
        drop(st);
        self.shared.room.notify_one();
        // After the end of output the thread has finished or is about to:
        // wait for it, so it doesn't outlive the session. Before it, it
        // ends at the program's exit or its next read; don't wait for that.
        if let Some(t) = self.thread.take()
            && ended
        {
            let _ = t.join();
        }
    }
}

fn read_loop(reader: BatchReader, shared: &Shared) {
    use std::io::Read;
    // Like the parser's thread, on a performance core and woken at once: a
    // program writing a flood waits on this thread every kilobyte.
    // SAFETY: sets the calling thread's own class; no pointers.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0);
    }
    let mut file = &reader.file;
    let fd = file.as_raw_fd();
    let mut chunk = [0u8; PTY_READ_MAX * 4];
    let mut flooding = false;
    // Bytes buffered that a waiting parser hasn't been told about.
    let mut unannounced = false;
    let announce = |shared: &Shared| {
        let st = shared.lock();
        let wake = st.parser_waiting && !st.buf.is_empty();
        drop(st);
        if wake {
            shared.data.notify_one();
        }
    };
    let mut spin_until: Option<Instant> = None;
    loop {
        match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                spin_until = None;
                flooding = n >= PTY_READ_MAX;
                let mut st = shared.lock();
                while st.buf.len() >= PUMP_CAP && !st.closed {
                    if st.parser_waiting {
                        shared.data.notify_one();
                    }
                    st.reader_waiting = true;
                    st = shared.room.wait(st).unwrap_or_else(PoisonError::into_inner);
                    st.reader_waiting = false;
                }
                if st.closed {
                    return;
                }
                st.buf.extend_from_slice(&chunk[..n]);
                let wake = st.parser_waiting && (!flooding || st.buf.len() >= PUMP_WAKE);
                unannounced = st.parser_waiting && !wake;
                drop(st);
                if wake {
                    shared.data.notify_one();
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                // Nothing to read: whatever is buffered goes to the parser
                // before this thread looks again or sleeps.
                if unannounced {
                    unannounced = false;
                    announce(shared);
                }
                if flooding {
                    let until = *spin_until.get_or_insert_with(|| Instant::now() + FLOOD_SPIN);
                    if Instant::now() < until {
                        std::hint::spin_loop();
                        continue;
                    }
                    flooding = false;
                }
                spin_until = None;
                if !wait_fd(fd, libc::POLLIN) {
                    break;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            // EIO: the program and everything it started have exited.
            Err(_) => break,
        }
    }
    shared.lock().eof = true;
    shared.data.notify_one();
}
