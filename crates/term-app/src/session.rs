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
//!
//! Failures are shown in the terminal, since an app started from the Dock
//! has no stderr. A program that can't start, or that fails within
//! [`QUICK_EXIT`], leaves a message, and [`Outcome::Hold`] keeps the window
//! open. A panic while parsing (a bug in decline) is caught: the session
//! stops reading, kills the program and says where the crash log is, while
//! other windows keep running. The panic is caught while the terminal's
//! lock is still held, so the lock is never poisoned: the broken terminal
//! is replaced by a fresh one before anyone else can see it.

use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use term_core::Term;
use term_pty::{Child, Command, Pty, Size};

use crate::control::ControlScanner;

/// A macOS PTY hands out ~1 KB per read, so 64 KB is plenty.
const READ_BUF: usize = 64 * 1024;

/// A program that fails within this long keeps its window open, so its
/// error can be read.
pub const QUICK_EXIT: Duration = Duration::from_secs(1);

/// How long to wait for the exit status once the output has ended.
const STATUS_WAIT: Duration = Duration::from_millis(500);

pub const PRESS_ANY_KEY: &str = "press any key to close";

/// With the `test-hooks` feature (used only by tests), output containing
/// this makes the parse step panic, to test that the panic is contained.
#[cfg(feature = "test-hooks")]
pub const TEST_PANIC: &[u8] = b"\x1b]7770;test-panic\x07";

/// How the session ended, for the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The program exited normally: the window can close.
    Closed,
    /// The program couldn't start, failed straight away, or decline hit a
    /// bug. A message saying which is in the terminal, and the window stays
    /// open until a key is pressed.
    Hold,
}

/// Callbacks from the reader thread.
pub struct Hooks {
    /// Called when new output arrived and no redraw was pending.
    pub wake: Arc<dyn Fn() + Send + Sync>,
    /// Called once: when the program and everything it started have exited,
    /// when the session stopped because of a bug, or (from `spawn` itself)
    /// when the program couldn't be started.
    pub on_exit: Box<dyn FnOnce(Outcome) + Send>,
    /// Test-only: called with each private control command found in the
    /// output (see [`crate::control`]), after the output before it has been
    /// parsed. `None`, the normal case, doesn't look for them.
    pub on_control: Option<Box<dyn FnMut(String) + Send>>,
    /// Latency harness only: called after each parse with the terminal
    /// still locked (see [`crate::latency`]).
    pub on_parsed: Option<Box<dyn Fn() + Send>>,
    /// Called after a parse that produced [`Events`], which are taken out
    /// of the terminal. Unlike drawing, this happens while the window is
    /// hidden too (a background tab's title is in the tab bar).
    pub on_events: Option<Box<dyn Fn(Events) + Send>>,
}

impl Hooks {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>, on_exit: Box<dyn FnOnce(Outcome) + Send>) -> Hooks {
        Hooks {
            wake,
            on_exit,
            on_control: None,
            on_parsed: None,
            on_events: None,
        }
    }
}

/// What the program asked for that the window acts on whether or not it
/// is drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Events {
    /// A new title (OSC 0/2).
    pub title: Option<String>,
    pub bell: bool,
    /// OSC 9 / OSC 777.
    pub notification: Option<term_core::Notification>,
    /// OSC 9;4.
    pub progress: Option<term_core::Progress>,
}

impl Events {
    fn take(t: &mut Term) -> Events {
        Events {
            title: t.state.take_title(),
            bell: t.state.take_bell(),
            notification: t.state.take_notification(),
            progress: t.state.take_progress(),
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Events::default()
    }
}

/// Write `text` into `term` as a dim red line from decline itself.
pub fn write_notice(term: &mut Term, text: &str) {
    write_red(term, &format!("decline: {text}"));
}

/// A notice written while a program runs (after a config reload): on a line
/// of its own, after whatever the program printed.
pub fn write_late_notice(term: &mut Term, text: &str) {
    if term.state.cursor_in_view().is_none_or(|(_, col)| col > 0) {
        term.advance(b"\r\n");
    }
    write_notice(term, text);
}

fn write_red(term: &mut Term, line: &str) {
    term.advance(format!("\x1b[0;2;31m{line}\x1b[0m\r\n").as_bytes());
    term.state.take_replies();
}

fn write_dim(term: &mut Term, text: &str) {
    term.advance(format!("\x1b[0;2m{text}\x1b[0m").as_bytes());
    term.state.take_replies();
}

/// The terminal's lock. Panics while parsing are caught with the lock held
/// (see [`parse`]), so they never poison it; should anything else, the
/// state is used as it is rather than taking the app down.
pub fn lock(term: &Mutex<Term>) -> MutexGuard<'_, Term> {
    term.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Decline hit a bug while parsing.
struct Bug;

/// Parse `bytes` into the terminal, returning replies for the program and,
/// if `events` is set, the [`Events`] taken out of it. A panic is caught
/// before the lock is released, and the terminal, possibly half-updated, is
/// replaced by a fresh one of the same size.
fn parse(term: &Mutex<Term>, bytes: &[u8], on_parsed: Option<&(dyn Fn() + Send)>, events: bool) -> Result<(Vec<u8>, Events), Bug> {
    let mut t = lock(term);
    let parsed = catch_unwind(AssertUnwindSafe(|| {
        test_panic(bytes);
        t.advance(bytes);
    }));
    if let Some(f) = on_parsed {
        f();
    }
    match parsed {
        Ok(()) => {
            let events = if events { Events::take(&mut t) } else { Events::default() };
            Ok((t.state.take_replies(), events))
        }
        Err(_) => {
            replace_term(&mut t);
            Err(Bug)
        }
    }
}

fn replace_term(t: &mut Term) {
    let mut fresh = Term::new(t.state.cols(), t.state.lines(), 0);
    fresh.state.set_default_colors(t.state.default_colors());
    fresh.state.set_cell_pixels(t.state.cell_pixels());
    *t = fresh;
}

#[cfg(feature = "test-hooks")]
fn test_panic(bytes: &[u8]) {
    if bytes.windows(TEST_PANIC.len()).any(|w| w == TEST_PANIC) {
        panic!("test-hooks: the parse step was asked to panic");
    }
}

#[cfg(not(feature = "test-hooks"))]
#[inline(always)]
fn test_panic(_bytes: &[u8]) {}

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
    ///
    /// If the program can't be started, the session has no program: the
    /// error is written into `term` and `on_exit(Outcome::Hold)` is called
    /// before this returns. Errors are only for failing to start threads.
    pub fn spawn(command: &Command, size: Size, term: Term, hooks: Hooks) -> io::Result<Session> {
        let started = Instant::now();
        let term = Arc::new(Mutex::new(term));
        let dirty = Arc::new(AtomicBool::new(true));
        let pty = match Pty::spawn(command, size) {
            Ok(pty) => pty,
            Err(e) => {
                let what = command.argv.first().map(|p| format!("{p:?}")).unwrap_or_else(|| "the shell".into());
                eprintln!("decline: could not start {what}: {e}");
                {
                    let mut t = lock(&term);
                    write_notice(&mut t, &format!("could not start {what}: {e}"));
                    write_dim(&mut t, PRESS_ANY_KEY);
                }
                (hooks.on_exit)(Outcome::Hold);
                return Ok(Session {
                    pty: None,
                    term,
                    dirty,
                    input: None,
                    reader: None,
                    writer: None,
                });
            }
        };

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
        let child = pty.child();
        let (term2, dirty2, tx2) = (term.clone(), dirty.clone(), tx.clone());
        let Hooks {
            wake,
            on_exit,
            mut on_control,
            on_parsed,
            on_events,
        } = hooks;
        let reader = std::thread::Builder::new().name("pty-reader".into()).spawn(move || {
            // A panic anywhere else in the loop stops the session too.
            let read = catch_unwind(AssertUnwindSafe(|| -> Result<(), Bug> {
                let mut buf = vec![0u8; READ_BUF];
                let mut scanner = on_control.as_ref().map(|_| ControlScanner::default());
                let mut found = Vec::new();
                let parse = |bytes: &[u8]| -> Result<(), Bug> {
                    if bytes.is_empty() {
                        return Ok(());
                    }
                    let (replies, events) = parse(&term2, bytes, on_parsed.as_deref(), on_events.is_some())?;
                    if !replies.is_empty() {
                        let _ = tx2.send(replies);
                    }
                    if let Some(f) = &on_events
                        && !events.is_empty()
                    {
                        f(events);
                    }
                    Ok(())
                };
                loop {
                    let n = reader.read_batch(&mut buf);
                    if n == 0 {
                        return Ok(());
                    }
                    let mut start = 0;
                    if let (Some(scanner), Some(control)) = (&mut scanner, &mut on_control) {
                        scanner.feed(&buf[..n], &mut found);
                        for (end, cmd) in found.drain(..) {
                            parse(&buf[start..end])?;
                            start = end;
                            control(cmd);
                        }
                    }
                    parse(&buf[start..n])?;
                    if !dirty2.swap(true, Ordering::AcqRel) {
                        wake();
                    }
                }
            }));
            let outcome = match read {
                Ok(Ok(())) => finished(&term2, &child, started),
                Ok(Err(Bug)) | Err(_) => {
                    child.signal(libc::SIGKILL, true);
                    show_bug(&term2);
                    // Keep reading (and dropping) output until the program
                    // is gone: on macOS a process can't finish exiting while
                    // its terminal holds output nobody reads.
                    let mut sink = [0u8; 4096];
                    while reader.read_batch(&mut sink) > 0 {}
                    Outcome::Hold
                }
            };
            if outcome == Outcome::Hold && !dirty2.swap(true, Ordering::AcqRel) {
                wake();
            }
            on_exit(outcome);
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

    /// The terminal, locked (see [`lock`]).
    pub fn lock_term(&self) -> MutexGuard<'_, Term> {
        lock(&self.term)
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
            let mut t = lock(&self.term);
            t.state.resize(size.cols as usize, size.lines as usize);
            t.state.set_cell_pixels((size.cell_width, size.cell_height));
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
    pub fn shut_down(self) {
        for h in self.hang_up() {
            if let Err(e) = h.join() {
                std::panic::resume_unwind(e);
            }
        }
    }

    /// What dropping the session does, handing back the reader and writer
    /// threads (in that order, those still running) so a caller can check
    /// they finish.
    pub fn hang_up(mut self) -> Vec<JoinHandle<()>> {
        self.release();
        [self.reader.take(), self.writer.take()].into_iter().flatten().collect()
    }

    /// Hangs up on the program (SIGHUP; term-pty kills its process group if
    /// it is still there after a grace period). The reader then sees the end
    /// of output and exits, and the writer once both senders are gone.
    fn release(&mut self) {
        self.pty = None;
        self.input = None;
    }
}

/// The output has ended. A program that failed within [`QUICK_EXIT`] keeps
/// the window open with its status; anything else closes it.
fn finished(term: &Mutex<Term>, child: &Child, started: Instant) -> Outcome {
    use std::os::unix::process::ExitStatusExt;
    if started.elapsed() >= QUICK_EXIT {
        return Outcome::Closed;
    }
    // The output ends when the program closes the terminal, which is
    // usually just before it exits.
    let until = Instant::now() + STATUS_WAIT;
    let status = loop {
        match child.try_wait() {
            Some(s) => break s,
            None if Instant::now() >= until => return Outcome::Closed,
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    let what = match (status.code(), status.signal()) {
        (Some(0), _) | (None, None) => return Outcome::Closed,
        (Some(code), _) => format!("[process exited with status {code}]"),
        (None, Some(sig)) => format!("[process killed by signal {sig}]"),
    };
    let mut t = lock(term);
    t.state.scroll_view_to_bottom();
    write_dim(&mut t, &format!("\r\n{what}\r\n{PRESS_ANY_KEY}"));
    Outcome::Hold
}

/// Decline hit a bug in this session: say so, and where the details are.
fn show_bug(term: &Mutex<Term>) {
    let log = crate::crash::take_last_log();
    let mut t = lock(term);
    // After a panic outside `parse` the terminal may be half-updated too;
    // after one inside, this is already a fresh one.
    replace_term(&mut t);
    let msg = match log {
        Some(p) => format!("decline hit a bug and stopped this session; details in {}", crate::crash::tilde(&p)),
        None => "decline hit a bug and stopped this session; details on stderr".into(),
    };
    write_red(&mut t, &msg);
    write_dim(&mut t, PRESS_ANY_KEY);
}

impl Drop for Session {
    fn drop(&mut self) {
        // Not joined: that could keep the main thread waiting for a program
        // that ignores SIGHUP.
        self.release();
    }
}
