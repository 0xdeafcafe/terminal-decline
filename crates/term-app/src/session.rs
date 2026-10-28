//! A running program and its terminal, without any AppKit: the PTY, the
//! reader and writer threads, the shared [`Term`] and the dirty flag. The
//! window's view uses this; so do the headless pipeline tests.
//!
//! Threads:
//! - reader: one `read_batch` per loop, parse it, queue any replies for the
//!   writer, then set the dirty flag and call `wake` only if it was clear (so
//!   a flood of output wakes the main thread at most once per frame). When
//!   the program has exited it calls `on_exit`.
//! - writer: writes typed input and query replies to the PTY, waiting for
//!   room there rather than on the main thread.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use term_core::Term;
use term_pty::{Command, Pty, Size};

/// A macOS PTY hands out ~1 KB per read, so 64 KB is plenty.
const READ_BUF: usize = 64 * 1024;

/// Callbacks from the reader thread.
pub struct Hooks {
    /// Called when new output arrived and no redraw was pending.
    pub wake: Arc<dyn Fn() + Send + Sync>,
    /// Called once, when the program and everything it started have exited.
    pub on_exit: Box<dyn FnOnce() + Send>,
}

impl Hooks {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>, on_exit: Box<dyn FnOnce() + Send>) -> Hooks {
        Hooks { wake, on_exit }
    }
}

pub struct Session {
    pty: Option<Pty>,
    term: Arc<Mutex<Term>>,
    dirty: Arc<AtomicBool>,
    input: Option<Sender<Vec<u8>>>,
    reader: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
}

impl Session {
    /// Start `command` on a PTY of `size`, feeding its output into `term`.
    pub fn spawn(command: &Command, size: Size, term: Term, hooks: Hooks) -> io::Result<Session> {
        let pty = Pty::spawn(command, size)?;
        let term = Arc::new(Mutex::new(term));
        let dirty = Arc::new(AtomicBool::new(true));

        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let mut writer = pty.writer()?;
        let writer = std::thread::Builder::new().name("pty-writer".into()).spawn(move || {
            for bytes in rx {
                if writer.write_all(&bytes).is_err() {
                    break;
                }
            }
        })?;

        let mut reader = pty.reader()?;
        let (term2, dirty2, tx2) = (term.clone(), dirty.clone(), tx.clone());
        let Hooks { wake, on_exit } = hooks;
        let reader = std::thread::Builder::new().name("pty-reader".into()).spawn(move || {
            let mut buf = vec![0u8; READ_BUF];
            let parse = |bytes: &[u8]| {
                if bytes.is_empty() {
                    return;
                }
                let replies = {
                    let mut t = term2.lock().unwrap();
                    t.advance(bytes);
                    std::mem::take(&mut t.state.replies)
                };
                if !replies.is_empty() {
                    let _ = tx2.send(replies);
                }
            };
            loop {
                let n = reader.read_batch(&mut buf);
                if n == 0 {
                    break;
                }
                parse(&buf[..n]);
                if !dirty2.swap(true, Ordering::AcqRel) {
                    wake();
                }
            }
            on_exit();
        })?;

        Ok(Session {
            pty: Some(pty),
            term,
            dirty,
            input: Some(tx),
            reader: Some(reader),
            writer: Some(writer),
        })
    }

    pub fn term(&self) -> &Arc<Mutex<Term>> {
        &self.term
    }

    /// Whether anything changed since the last call; clears the flag.
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::AcqRel)
    }

    /// Ask for a redraw. Doesn't wake anyone: the caller is the one drawing.
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
    }

    /// Send bytes to the program (in order, on the writer thread).
    pub fn send(&self, bytes: Vec<u8>) {
        if let Some(tx) = &self.input {
            let _ = tx.send(bytes);
        }
    }

    /// Resize the terminal and tell the program (SIGWINCH).
    pub fn resize(&self, size: Size) -> io::Result<()> {
        {
            let mut t = self.term.lock().unwrap();
            t.state.resize(size.cols as usize, size.lines as usize);
            t.state.cell_pixels = (size.cell_width, size.cell_height);
        }
        self.mark_dirty();
        match &self.pty {
            Some(pty) => pty.resize(size),
            None => Ok(()),
        }
    }

    pub fn pid(&self) -> Option<libc::pid_t> {
        self.pty.as_ref().map(Pty::pid)
    }

    /// The working directory of the program in the foreground.
    pub fn foreground_cwd(&self) -> Option<String> {
        self.pty.as_ref()?.foreground_cwd()
    }

    /// Wait for the program to exit and the reader thread to finish,
    /// re-raising a panic from the reader thread.
    pub fn wait(&mut self) {
        if let Some(h) = self.reader.take()
            && let Err(e) = h.join()
        {
            std::panic::resume_unwind(e);
        }
    }

    /// Hang up on the program and wait for both threads to finish,
    /// re-raising a panic from either.
    pub fn shut_down(mut self) {
        self.pty = None;
        self.wait();
        self.input = None;
        if let Some(h) = self.writer.take()
            && let Err(e) = h.join()
        {
            std::panic::resume_unwind(e);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Hangs up on the program; the reader sees the end of output and
        // exits, then the writer once both senders are gone. Not joined: that
        // could keep the main thread waiting for a program ignoring SIGHUP.
        self.pty = None;
        self.input = None;
    }
}
