//! A running program and its terminal, without any AppKit: the PTY, the
//! reader and writer threads, the shared [`Term`] and the dirty flag. The
//! window's view uses this; so do the headless pipeline tests.
//!
//! Threads:
//! - pty-read: reads the PTY into a bounded buffer ([`term_pty::Pump`]), so
//!   the reads of a flood overlap its parse.
//! - reader: takes what pty-read has read, parses it (in pieces of at most
//!   [`PARSE_MAX`], one lock each), queues any replies for the writer, then
//!   sets the dirty flag and calls `wake` only if it was clear (so a flood of
//!   output wakes the main thread at most once per frame). When the program
//!   has exited it calls `on_exit`.
//! - writer: writes typed input and query replies to the PTY, waiting for
//!   room there rather than on the main thread.
//! - prompt-wait, only for a command typed for a restored tab
//!   ([`Session::type_at_prompt`]): waits for the shell's line editor,
//!   types the command and ends.
//!
//! Failures are shown in the terminal, since an app started from the Dock
//! has no stderr. A program that can't start, or that fails within
//! [`QUICK_EXIT`], leaves a message, and [`Outcome::Hold`] keeps the window
//! open. A panic while parsing (a bug in Terminal Decline) is caught: the
//! session stops reading, kills the program and says where the crash log is,
//! while other windows keep running. The panic is caught while the terminal's
//! lock is still held, so the lock is never poisoned: the broken terminal is
//! replaced by a fresh one before anyone else can see it.

use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use term_core::Term;
use term_pty::{Child, Command, Pty, Size};

use crate::control::ControlScanner;
use crate::images::SessionImages;

/// The most output parsed under one hold of the terminal's lock: ~100 µs
/// of coloured text, well inside the frame's [`FRAME_LOCK_WAIT`].
const PARSE_MAX: usize = 16 * 1024;

/// A program that fails within this long keeps its window open, so its
/// error can be read.
pub const QUICK_EXIT: Duration = Duration::from_secs(1);

/// How long to wait for the exit status once the output has ended.
const STATUS_WAIT: Duration = Duration::from_millis(500);

pub const PRESS_ANY_KEY: &str = "press any key to close";

/// How often [`Session::type_at_prompt`] looks whether the shell's line
/// editor is reading, and how long the output must have been quiet.
const PROMPT_CHECK: Duration = Duration::from_millis(50);

/// How long [`Session::type_at_prompt`] waits for a prompt before giving up.
const PROMPT_WAIT: Duration = Duration::from_secs(30);

/// With the `test-hooks` feature (used only by tests), output containing
/// this makes the parse step panic, to test that the panic is contained.
#[cfg(feature = "test-hooks")]
pub const TEST_PANIC: &[u8] = b"\x1b]7770;test-panic\x07";

/// How the session ended, for the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The program exited normally: the window can close.
    Closed,
    /// The program couldn't start, failed straight away, or Terminal Decline
    /// hit a bug. A message saying which is in the terminal, and the window
    /// stays open until a key is pressed.
    Hold,
}

/// See [`Hooks::on_output`].
pub type OnOutput = Box<dyn FnMut(&[u8], Duration) + Send>;

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
    /// The Settings tab only: called with what follows `OSC 7771 ;` for
    /// each such sequence in the output, the preferences asking the app
    /// to record a chord (`record-chord`) or to preview a theme
    /// (`theme-preview;…`). Set by the app when it starts the preferences
    /// itself, so no other program can ask. `None` ignores the sequences
    /// like any unknown OSC.
    pub on_prefs: Option<Box<dyn FnMut(String) + Send>>,
    /// The agent API's sessions ([`crate::ctl`]): called after each batch
    /// of output has been parsed, with the batch and the time parsing it
    /// took.
    pub on_output: Option<OnOutput>,
    /// Don't write "[process exited with status N]" when the program
    /// fails within [`QUICK_EXIT`]: the agent API reports the status
    /// itself.
    pub quiet_exit: bool,
}

impl Hooks {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>, on_exit: Box<dyn FnOnce(Outcome) + Send>) -> Hooks {
        Hooks {
            wake,
            on_exit,
            on_control: None,
            on_parsed: None,
            on_events: None,
            on_prefs: None,
            on_output: None,
            quiet_exit: false,
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
    /// Commands started and finished (OSC 133 C and D), oldest first.
    pub commands: Vec<term_core::Command>,
}

impl Events {
    fn take(t: &mut Term) -> Events {
        Events {
            title: t.state.take_title(),
            bell: t.state.take_bell(),
            notification: t.state.take_notification(),
            progress: t.state.take_progress(),
            commands: t.state.take_commands(),
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Events::default()
    }

    /// Fold `later` into these as if both had come in one parse slice: the
    /// latest title, notification and progress, the bell if either rang,
    /// and every command, in order (each finished one may notify).
    pub fn merge(&mut self, later: Events) {
        // Destructured so a new field can't be dropped here unnoticed.
        let Events {
            title,
            bell,
            notification,
            progress,
            commands,
        } = later;
        if title.is_some() {
            self.title = title;
        }
        self.bell |= bell;
        if notification.is_some() {
            self.notification = notification;
        }
        if progress.is_some() {
            self.progress = progress;
        }
        self.commands.extend(commands);
    }
}

/// Events on their way from a reader thread to the main thread. Setting a
/// window's title costs AppKit a millisecond or more, so one block per
/// parse slice let a program that sets the title in every slice queue up
/// more work than the main thread could do: it stopped drawing and taking
/// keys for many seconds after the output ended. Events that come while a
/// block is still queued join it instead.
#[derive(Debug, Default)]
pub struct PendingEvents(std::sync::Mutex<Option<Events>>);

impl PendingEvents {
    /// Add `events`. True when none were waiting: the caller then queues
    /// the block that will [`take`](Self::take) them.
    pub fn push(&self, events: Events) -> bool {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match &mut *slot {
            Some(waiting) => {
                waiting.merge(events);
                false
            }
            None => {
                *slot = Some(events);
                true
            }
        }
    }

    pub fn take(&self) -> Option<Events> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

/// Write `text` into `term` as a dim red line from Terminal Decline itself.
pub fn write_notice(term: &mut Term, text: &str) {
    write_red(term, &format!("terminal-decline: {text}"));
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

/// Run the calling thread (the PTY's reader or writer) as the window's
/// own: on a performance core, woken at once. A macOS PTY holds ~2 KB, so
/// a program writing a flood waits on the reader every kilobyte; at the
/// default class the reader is put on an efficiency core when the Mac is
/// busy, where each kilobyte takes longer and costs more CPU time.
fn interactive() {
    // SAFETY: sets the calling thread's own class; no pointers.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0);
    }
}

/// The terminal's lock. Panics while parsing are caught with the lock held
/// (see [`parse`]), so they never poison it; should anything else, the
/// state is used as it is rather than taking the app down.
pub fn lock(term: &Mutex<Term>) -> MutexGuard<'_, Term> {
    term.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The longest a frame waits for a terminal's lock (roadmap R.1).
pub const FRAME_LOCK_WAIT: Duration = Duration::from_micros(500);

/// The terminal's lock for taking a frame's snapshot, on the main thread:
/// tried, then tried again for up to [`FRAME_LOCK_WAIT`]. The reader
/// holds it for one read's parse (a macOS PTY read is ~1 KB: microseconds),
/// so this nearly always gets it at once; on a loaded Mac the reader can
/// be taken off its core while parsing, and the main thread must not wait
/// on that. `None`: still busy; the pane draws its last frame and keeps
/// its change for the next tick.
pub fn lock_for_frame(term: &Mutex<Term>) -> Option<MutexGuard<'_, Term>> {
    let mut started = None;
    loop {
        match term.try_lock() {
            Ok(t) => return Some(t),
            Err(std::sync::TryLockError::Poisoned(p)) => return Some(p.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => {}
        }
        if started.get_or_insert_with(Instant::now).elapsed() >= FRAME_LOCK_WAIT {
            return None;
        }
        // Spun, not yielded: on a loaded Mac a yield can give the core
        // away for a whole time slice (10 ms measured).
        std::hint::spin_loop();
    }
}

/// Milliseconds since the first call, the clock commands are timed by.
fn clock_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

/// Terminal Decline hit a bug while parsing.
struct Bug;

/// Parse `bytes` into the terminal, returning replies for the program and,
/// if `events` is set, the [`Events`] taken out of it. A panic is caught
/// before the lock is released, and the terminal, possibly half-updated, is
/// replaced by a fresh one of the same size.
/// Parse `bytes`: the replies to send, the events, and whether the bytes
/// held only the shell's line reports (OSC 7701) and nothing to draw.
fn parse(
    term: &Mutex<Term>,
    bytes: &[u8],
    on_parsed: Option<&(dyn Fn() + Send)>,
    events: bool,
    images: Option<&SessionImages>,
) -> Result<(Vec<u8>, Events, bool), Bug> {
    let mut t = lock(term);
    // For timing commands (OSC 133 C to D): term-core reads no clock.
    t.state.set_clock(clock_ms());
    let generation = t.state.generation();
    let (mut t, parsed) = if images.is_some() {
        parse_with_head_reads(term, t, bytes)
    } else {
        let parsed = catch_unwind(AssertUnwindSafe(|| {
            test_panic(bytes);
            t.advance(bytes);
        }));
        (t, parsed)
    };
    if let Some(f) = on_parsed {
        f();
    }
    match parsed {
        Ok(()) => {
            if let Some(images) = images {
                // Queued with the lock held, run with it released.
                images.take_jobs(&mut t.state);
            }
            let events = if events { Events::take(&mut t) } else { Events::default() };
            let only_reports = t.state.take_report_seen() && t.state.generation() == generation;
            Ok((t.state.take_replies(), events, only_reports))
        }
        Err(_) => {
            replace_term(&mut t);
            Err(Bug)
        }
    }
}

/// Parse with images on: when a kitty command needs a medium's header
/// (the size of a PNG from a file placed at once, images.md §3.4), the
/// parser stops there, the lock is let go while the host reads it, and
/// parsing carries on from where it stopped. Returns the guard, held
/// again. A panic is caught with the lock held, as in [`parse`]. Should
/// the terminal be replaced meanwhile, the new one ignores the header.
pub(crate) fn parse_with_head_reads<'a>(
    term: &'a Mutex<Term>,
    mut t: MutexGuard<'a, Term>,
    bytes: &[u8],
) -> (MutexGuard<'a, Term>, std::thread::Result<()>) {
    if let Err(e) = catch_unwind(|| test_panic(bytes)) {
        return (t, Err(e));
    }
    let mut rest = bytes;
    while !rest.is_empty() {
        let step = catch_unwind(AssertUnwindSafe(|| {
            let n = t.advance_until_head(rest);
            (n, t.state.take_head_read())
        }));
        let (n, read) = match step {
            Ok(s) => s,
            Err(e) => return (t, Err(e)),
        };
        rest = &rest[n..];
        let Some(read) = read else {
            if n == 0 {
                // Stopped with no header to read: parse the rest as usual.
                let r = catch_unwind(AssertUnwindSafe(|| t.advance(rest)));
                return (t, r);
            }
            continue;
        };
        let host = t.state.image_host().clone();
        drop(t);
        // Unlocked: the read may wait on the disk.
        let head = host.read_head(read.medium(), read.max);
        t = lock(term);
        if let Err(e) = catch_unwind(AssertUnwindSafe(|| t.state.head_read_done(read, head))) {
            return (t, Err(e));
        }
    }
    (t, Ok(()))
}

/// Replace a terminal that may be half-updated by a panic with a fresh one
/// of the same size and colours.
pub(crate) fn replace_term(t: &mut Term) {
    // The same pool: the old terminal's images are freed when it is dropped.
    let mut fresh = Term::new(t.state.cols(), t.state.lines(), 0, t.state.image_pool().clone());
    fresh.state.set_default_colors(t.state.default_colors());
    fresh.state.set_default_palette(t.state.default_palette());
    fresh.state.set_cell_pixels(t.state.cell_pixels());
    fresh.state.set_image_scale(t.state.image_scale());
    // The same host: the app's.
    fresh.state.set_image_host(t.state.image_host().clone());
    fresh.state.set_reads_off_lock(true);
    fresh.state.set_report_token(t.state.report_token());
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
    /// Batches of output read so far.
    batches: Arc<AtomicU64>,
    /// Stopped for a bug outside the reader ([`Session::stop_for_bug`]):
    /// output is dropped unparsed, and the end of it holds the window
    /// without a message of its own.
    stopped: Arc<AtomicBool>,
    /// Where a command waiting for the prompt is sent
    /// ([`Session::type_at_prompt`]); taken when it is sent or given up
    /// on, or when something else is typed first.
    at_prompt: Arc<Mutex<Option<Sender<Vec<u8>>>>>,
    /// Set by [`Session::send`], which is what the user typing, pasting
    /// or clicking into the pane goes through (the agent API's idle limit
    /// leaves a window the user typed into alone).
    typed: Arc<AtomicBool>,
    /// Its side of the decode queue, when images are on.
    images: Option<Arc<SessionImages>>,
}

impl Session {
    /// Start `command` on a PTY of `size`, feeding its output into `term`.
    ///
    /// If the program can't be started, the session has no program: the
    /// error is written into `term` and `on_exit(Outcome::Hold)` is called
    /// before this returns. Errors are only for failing to start threads.
    pub fn spawn(command: &Command, size: Size, mut term: Term, hooks: Hooks) -> io::Result<Session> {
        let started = Instant::now();
        // Always, so the `images` setting can be turned on while the
        // session runs: Metal pixels, the app's media rules, and headers
        // read with the lock released. (With images off, none of it is
        // reached.)
        term.state.set_image_host(crate::images::process_host());
        term.state.set_reads_off_lock(true);
        let terminal_id = term.state.terminal_id();
        let term = Arc::new(Mutex::new(term));
        let dirty = Arc::new(AtomicBool::new(true));
        let pty = match Pty::spawn(command, size) {
            Ok(pty) => pty,
            Err(e) => {
                let what = command.argv.first().map(|p| format!("{p:?}")).unwrap_or_else(|| "the shell".into());
                eprintln!("terminal-decline: could not start {what}: {e}");
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
                    batches: Arc::default(),
                    stopped: Arc::default(),
                    at_prompt: Arc::default(),
                    typed: Arc::default(),
                    images: None,
                });
            }
        };

        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let mut writer = pty.writer()?;
        let writer = std::thread::Builder::new().name("pty-writer".into()).spawn(move || {
            interactive();
            for bytes in rx {
                if writer.write_all(&bytes).is_err() {
                    break;
                }
            }
        })?;

        let mut pump = term_pty::Pump::start(pty.reader()?)?;
        let child = pty.child();
        let (term2, dirty2, tx2) = (term.clone(), dirty.clone(), tx.clone());
        let batches = Arc::new(AtomicU64::new(0));
        let batches2 = batches.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let stopped2 = stopped.clone();
        let Hooks {
            wake,
            on_exit,
            mut on_control,
            on_parsed,
            on_events,
            mut on_prefs,
            mut on_output,
            quiet_exit,
        } = hooks;
        let images = Some(SessionImages::for_process(&term, terminal_id, dirty.clone(), wake.clone()));
        let images2 = images.clone();
        let reader = std::thread::Builder::new().name("pty-reader".into()).spawn(move || {
            interactive();
            // A panic anywhere else in the loop stops the session too.
            let read = catch_unwind(AssertUnwindSafe(|| -> Result<(), Bug> {
                let mut batch = Vec::with_capacity(term_pty::PUMP_CAP);
                let mut scanner = on_control.as_ref().map(|_| ControlScanner::default());
                let mut prefs = on_prefs.as_ref().map(|_| ControlScanner::new(crate::control::PREFS_PREFIX));
                let mut asked = Vec::new();
                let mut found = Vec::new();
                // Whether to wake the view for what was parsed: not for
                // the shell's line reports alone. zsh writes one just
                // before its redraw of the line, which the reader often
                // reads apart; waking for it would draw nothing new and
                // stop the view waiting for the echo, which would then
                // wait for the next tick (docs/design/completion.md §3.2).
                let parse = |bytes: &[u8]| -> Result<bool, Bug> {
                    if bytes.is_empty() {
                        return Ok(false);
                    }
                    if stopped2.load(Ordering::Acquire) {
                        return Ok(true);
                    }
                    let (replies, events, only_reports) =
                        parse(&term2, bytes, on_parsed.as_deref(), on_events.is_some(), images2.as_deref())?;
                    if !replies.is_empty() {
                        let _ = tx2.send(replies);
                    }
                    let wake = !only_reports || !events.is_empty();
                    if let Some(f) = &on_events
                        && !events.is_empty()
                    {
                        f(events);
                    }
                    Ok(wake)
                };
                loop {
                    if let Some(images) = &images2 {
                        // Decoding lags: let the PTY slow the program down.
                        images.hold_back();
                    }
                    if !pump.next(&mut batch) {
                        return Ok(());
                    }
                    for buf in batch.chunks(PARSE_MAX) {
                        let n = buf.len();
                        batches2.fetch_add(1, Ordering::Relaxed);
                        let parse_started = on_output.as_ref().map(|_| Instant::now());
                        let mut start = 0;
                        let mut wake_view = false;
                        if let (Some(scanner), Some(control)) = (&mut scanner, &mut on_control) {
                            scanner.feed(&buf[..n], &mut found);
                            for (end, cmd) in found.drain(..) {
                                parse(&buf[start..end])?;
                                start = end;
                                control(cmd);
                                wake_view = true;
                            }
                        }
                        wake_view |= parse(&buf[start..n])?;
                        if let (Some(f), Some(t0)) = (&mut on_output, parse_started) {
                            f(&buf[..n], t0.elapsed());
                        }
                        if let (Some(scanner), Some(ask)) = (&mut prefs, &mut on_prefs) {
                            scanner.feed(&buf[..n], &mut asked);
                            for (_, what) in asked.drain(..) {
                                ask(what);
                                wake_view = true;
                            }
                        }
                        if wake_view && !dirty2.swap(true, Ordering::AcqRel) {
                            wake();
                        }
                    }
                }
            }));
            let outcome = match read {
                // The message is in the terminal already.
                Ok(Ok(())) if stopped2.load(Ordering::Acquire) => Outcome::Hold,
                Ok(Ok(())) if quiet_exit => Outcome::Closed,
                Ok(Ok(())) => finished(&term2, &child, started),
                Ok(Err(Bug)) | Err(_) => {
                    child.signal(libc::SIGKILL, true);
                    show_bug(&term2);
                    // Keep reading (and dropping) output until the program
                    // is gone: on macOS a process can't finish exiting while
                    // its terminal holds output nobody reads.
                    while pump.next(&mut Vec::new()) {}
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
            batches,
            stopped,
            at_prompt: Arc::default(),
            typed: Arc::default(),
            images,
        })
    }

    /// Queue the image jobs `t` (this session's terminal, locked) has
    /// waiting: after a snapshot, so an idle terminal's re-decodes run.
    pub fn take_image_jobs(&self, t: &mut Term) {
        if let Some(images) = &self.images {
            images.take_jobs(&mut t.state);
        }
    }

    /// The terminal's id in the image pool, when images are on.
    pub fn image_owner(&self) -> Option<u64> {
        self.images.as_ref().map(|i| i.terminal_id())
    }

    /// Type `text` at the shell's prompt once its line editor is reading,
    /// without pressing Enter (a restored tab's command, for the user to
    /// start again). Typed before then, a shell may drop it or show it
    /// twice (the terminal echoes it, then the line editor draws it
    /// again). So this waits until the shell has written something and
    /// has the terminal reading keys unechoed ([`term_pty::Modes`]), with
    /// the output quiet for [`PROMPT_CHECK`]: zsh, bash and fish all do
    /// that at their prompt. It goes as a paste when the shell asked for
    /// bracketed paste (Enter still runs it), so key bindings and
    /// abbreviations don't act on it. Given up after [`PROMPT_WAIT`], or
    /// when anything else is typed first.
    ///
    /// `text` must have no control characters (quote them, as
    /// [`crate::running::quote`] does): they would be dropped.
    pub fn type_at_prompt(&self, text: String) {
        let (Some(pty), Some(tx)) = (&self.pty, &self.input) else { return };
        let Ok(modes) = pty.modes() else { return };
        *lock_slot(&self.at_prompt) = Some(tx.clone());
        let (slot, batches, term) = (self.at_prompt.clone(), self.batches.clone(), self.term.clone());
        let started = Instant::now();
        let spawned = std::thread::Builder::new().name("prompt-wait".into()).spawn(move || {
            let mut seen = 0;
            loop {
                std::thread::sleep(PROMPT_CHECK);
                let mut slot = lock_slot(&slot);
                if slot.is_none() {
                    return;
                }
                if started.elapsed() > PROMPT_WAIT {
                    *slot = None;
                    return;
                }
                let now = batches.load(Ordering::Relaxed);
                let quiet = now > 0 && now == seen;
                seen = now;
                if quiet && modes.reading_keys() {
                    let (bracketed, ours) = {
                        let t = lock(&term);
                        (t.state.modes().bracketed_paste, t.state.line_report().is_some())
                    };
                    if let Some(tx) = slot.take() {
                        let _ = tx.send(restore_bytes(&text, bracketed, ours));
                    }
                    return;
                }
            }
        });
        if spawned.is_err() {
            *lock_slot(&self.at_prompt) = None;
        }
    }

    /// Terminal Decline hit a bug drawing this session (splits design §7):
    /// its program is killed and its output from now on dropped, and the
    /// session ends held, with whatever message the caller wrote.
    pub fn stop_for_bug(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(pty) = &self.pty {
            pty.child().signal(libc::SIGKILL, true);
        }
        self.input = None;
        *lock_slot(&self.at_prompt) = None;
    }

    pub fn term(&self) -> &Arc<Mutex<Term>> {
        &self.term
    }

    /// A way to send bytes to the program from another thread, as
    /// [`Session::send`] does (without clearing a command waiting for
    /// the prompt): the agent API's windows use it.
    /// Whether anything has been typed ([`Session::send`]) into the session.
    pub fn typed(&self) -> Arc<AtomicBool> {
        self.typed.clone()
    }

    pub fn input(&self) -> Option<Sender<Vec<u8>>> {
        self.input.clone()
    }

    /// The terminal, locked (see [`lock`]).
    pub fn lock_term(&self) -> MutexGuard<'_, Term> {
        lock(&self.term)
    }

    /// The count of output batches read, shared: it moves when output
    /// arrives (see [`crate::idle_filters::Quiet`]).
    pub fn batches(&self) -> Arc<AtomicU64> {
        self.batches.clone()
    }

    /// Whether anything changed since the last call; clears the flag.
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::AcqRel)
    }

    /// Ask for a redraw. Doesn't wake anyone: the caller is the one drawing.
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
    }

    /// Send bytes to the program (in order, on the writer thread). A
    /// command waiting for the prompt isn't typed any more.
    pub fn send(&self, bytes: Vec<u8>) {
        self.typed.store(true, Ordering::Release);
        *lock_slot(&self.at_prompt) = None;
        if let Some(tx) = &self.input {
            let _ = tx.send(bytes);
        }
    }

    /// Send a report the terminal made unasked (the colour scheme, mode
    /// 2031) to the program, as query replies are: not typing, so a
    /// command waiting for the prompt stays.
    pub fn report(&self, bytes: Vec<u8>) {
        if let Some(tx) = &self.input
            && !bytes.is_empty()
        {
            let _ = tx.send(bytes);
        }
    }

    /// Device pixels per point of the window the terminal is in (2 on a
    /// Retina display), so iTerm2 images count their sizes as points.
    pub fn set_image_scale(&self, scale: f32) {
        lock(&self.term).state.set_image_scale(scale);
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

    /// The program's process, for its exit status.
    pub fn child(&self) -> Option<term_pty::Child> {
        self.pty.as_ref().map(Pty::child)
    }

    pub fn pid(&self) -> Option<libc::pid_t> {
        self.pty.as_ref().map(Pty::pid)
    }

    /// The terminal's device path (`/dev/ttys004`), while it has a program.
    pub fn tty_name(&self) -> Option<&str> {
        self.pty.as_ref()?.tty_name()
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
        if let Some(images) = &self.images {
            images.cancel();
        }
        self.pty = None;
        self.input = None;
        *lock_slot(&self.at_prompt) = None;
    }
}

fn lock_slot<T>(slot: &Mutex<T>) -> MutexGuard<'_, T> {
    slot.lock().unwrap_or_else(PoisonError::into_inner)
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

/// Terminal Decline hit a bug in this session: say so, and where the details
/// are.
fn show_bug(term: &Mutex<Term>) {
    let log = crate::crash::take_last_log();
    let mut t = lock(term);
    // After a panic outside `parse` the terminal may be half-updated too;
    // after one inside, this is already a fresh one.
    replace_term(&mut t);
    write_bug(&mut t, log);
}

/// Write into `t` that Terminal Decline hit a bug and stopped this session,
/// and where the crash log `log` is.
pub(crate) fn write_bug(t: &mut Term, log: Option<std::path::PathBuf>) {
    let msg = match log {
        Some(p) => format!(
            "Terminal Decline hit a bug and stopped this session; details in {}",
            crate::crash::tilde(&p)
        ),
        None => "Terminal Decline hit a bug and stopped this session; details on stderr".into(),
    };
    write_red(t, &msg);
    write_dim(t, PRESS_ANY_KEY);
}

impl Drop for Session {
    fn drop(&mut self) {
        // Not joined: that could keep the main thread waiting for a program
        // that ignores SIGHUP.
        self.release();
    }
}

/// What types a restored tab's command at the prompt. With our zsh
/// integration reporting the line (`ours`), it goes to decline.zsh's
/// widget (`ESC [ 7701 ~ = <line> BEL`), which makes it the line as if
/// typed: as a paste zsh would draw it highlighted until it is edited,
/// which looks like a selection a click can't clear. Else a paste
/// (bracketed when asked for), so key bindings don't act on it. A line
/// too long for the widget goes as a paste too.
fn restore_bytes(text: &str, bracketed: bool, ours: bool) -> Vec<u8> {
    let mut encoded = String::with_capacity(text.len());
    for c in text.chars() {
        if c == '%' || c.is_ascii_control() {
            encoded.push_str(&format!("%{:02X}", c as u32));
        } else {
            encoded.push(c);
        }
    }
    if ours && encoded.chars().count() < 4000 {
        format!("\x1b[7701~={encoded}\x07").into_bytes()
    } else {
        crate::keys::paste_bytes(text, bracketed)
    }
}

#[cfg(test)]
mod restore_tests {
    #[test]
    fn a_restored_command_goes_to_our_widget_when_zsh_reports_lines() {
        assert_eq!(
            super::restore_bytes("echo 100%", true, true),
            b"\x1b[7701~=echo 100%25\x07".to_vec()
        );
        assert_eq!(super::restore_bytes("ls", true, false), crate::keys::paste_bytes("ls", true));
        let long = "x".repeat(5000);
        assert_eq!(super::restore_bytes(&long, false, true), crate::keys::paste_bytes(&long, false));
    }
}
