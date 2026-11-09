//! Keypress-to-present timing, for the latency harness (`scripts/latency`).
//!
//! Only when `DECLINE_LATENCY_LOG=<file>` is set, each key that sends bytes
//! to the program is followed through four times, all on the one host clock
//! (`CLOCK_UPTIME_RAW`, which `NSEvent`, `CACurrentMediaTime` and Metal's
//! presented times also use), in seconds:
//!
//! - `t0`: the key event's own timestamp (when it was made, before the
//!   window server delivered it);
//! - `t1`: the key's bytes handed to the writer thread;
//! - `t2`: the reader thread parsed the next output in that window, with the
//!   terminal still locked (for a program that echoes, the echo);
//! - `t3`: the frame showing it reached the glass (the drawable's
//!   `presentedTime`), plus on the way `frame_start` (that frame locked the
//!   terminal to build), `built` and `drawable` (built, it asked for its
//!   drawable, and `nextDrawable` returned), `commit` (sent to the GPU)
//!   and `gpu_done`.
//!
//! One line of JSON per key is appended to the file once its frame is
//! presented (see [`Key`]). Unset, the cost is one check of a `OnceLock`
//! per key, parse and frame.
//!
//! Two more variables, also read only with the log set:
//! - `DECLINE_LATENCY_HZ=60|120`: ask the display link for this rate on
//!   every frame, instead of the usual 120 after input / 60 while
//!   streaming;
//! - `DECLINE_LATENCY_INPUT=<fifo>`: commands for the harness, one per line
//!   (see [`Command`]): post a key event to this process, or open a window
//!   running a program (behind the key window).

use std::ffi::c_void;
use std::fs::File;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

pub const ENV: &str = "DECLINE_LATENCY_LOG";
pub const ENV_HZ: &str = "DECLINE_LATENCY_HZ";
pub const ENV_INPUT: &str = "DECLINE_LATENCY_INPUT";

/// The host clock, seconds: the clock of `NSEvent.timestamp`,
/// `CACurrentMediaTime()` and `MTLDrawable.presentedTime`.
pub fn now() -> f64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_UPTIME_RAW, &mut ts) };
    ts.tv_sec as f64 + ts.tv_nsec as f64 * 1e-9
}

/// One key, as written to the log.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Key {
    /// 1, 2, … in the order keys were sent.
    pub seq: u64,
    /// The window (view id) it was typed in.
    pub window: usize,
    /// Bytes sent to the program.
    pub bytes: usize,
    pub t0: f64,
    /// `keyDown:` got the event.
    pub key_down: f64,
    pub t1: f64,
    pub t2: Option<f64>,
    pub frame_start: Option<f64>,
    /// The frame was built and asked for its drawable, and had it
    /// (`nextDrawable` returned).
    pub built: Option<f64>,
    pub drawable: Option<f64>,
    pub commit: Option<f64>,
    pub gpu_done: Option<f64>,
    /// `None` if the frame was never shown (macOS dropped it).
    pub t3: Option<f64>,
    /// Display link rate asked for when the frame was drawn.
    pub hz: u16,
}

/// One frame drawn while the view scrolls, as written to the log (`kind`
/// "frame"): the scroll events it shows, how long it took to build, how
/// far the view moved, and when it reached the glass. Times are host
/// seconds like [`Key`]'s.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct ScrollFrame {
    pub kind: &'static str,
    pub window: usize,
    /// Scroll events handled since the last frame, and the first's and
    /// last's own timestamps.
    pub events: u32,
    pub t0_first: Option<f64>,
    pub t0_last: Option<f64>,
    /// The tick started drawing.
    pub frame_start: f64,
    /// The focused pane was built (not only drawn again, moved).
    pub built: bool,
    /// Snapshot under the lock (history rows read and decoded), and
    /// building from it, in µs.
    pub snapshot_us: f64,
    pub build_us: f64,
    /// Rows the snapshot copied (not cached), of `rows` (the grid's
    /// `lines`, and a row below while smooth scrolling) by `cols`.
    pub copied: u32,
    pub rows: u32,
    pub cols: u16,
    /// The cell height, device pixels.
    pub cell_h: u32,
    /// The view's whole-line offset after the frame, and the pixels the
    /// content is drawn higher than that.
    pub offset: usize,
    pub shift_px: i32,
    /// Drawn without some new glyphs (the glyph thread has them).
    pub deferred: bool,
    /// Time the build waited for the GPU before putting new glyphs in
    /// their textures, µs.
    pub upload_wait_us: f64,
    /// The longest a scroll event waited for the terminal's lock since
    /// the last frame, µs.
    pub lock_wait_us: f64,
    pub commit: Option<f64>,
    pub gpu_done: Option<f64>,
    pub t3: Option<f64>,
    pub hz: u16,
}

/// A display-link tick that drew nothing while the view scrolls (`kind`
/// "tick"), with the frame policy's decision.
#[derive(Clone, Debug, Serialize)]
pub struct Tick {
    pub kind: &'static str,
    pub window: usize,
    pub t: f64,
    pub decision: String,
}

/// The timing log. [`get`] returns it only when `DECLINE_LATENCY_LOG` is set.
pub struct Latency {
    hz: Option<u16>,
    /// Keys whose output hasn't been parsed yet, so a reader can skip the
    /// lock while nothing is waiting.
    waiting: AtomicUsize,
    inner: Mutex<Inner>,
}

struct Inner {
    seq: u64,
    pending: Vec<Key>,
    /// Scroll events not yet in a frame: (window, timestamp).
    scrolls: Vec<(usize, f64)>,
    /// Each window's last scroll event: (window, when handled).
    last_scroll: Vec<(usize, f64)>,
    /// The longest wait for a terminal's lock in a scroll event since the
    /// last frame, seconds.
    lock_wait: f64,
    out: Box<dyn Write + Send>,
}

static LATENCY: OnceLock<Option<Latency>> = OnceLock::new();

/// The log, if `DECLINE_LATENCY_LOG` is set.
#[inline]
pub fn get() -> Option<&'static Latency> {
    LATENCY.get_or_init(from_env).as_ref()
}

fn from_env() -> Option<Latency> {
    let path = std::env::var_os(ENV).filter(|p| !p.is_empty())?;
    let file = match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("terminal-decline: {ENV}: {}: {e}", PathBuf::from(&path).display());
            return None;
        }
    };
    let hz = std::env::var(ENV_HZ).ok().and_then(|s| s.parse().ok()).filter(|&h: &u16| h > 0);
    Some(Latency::new(Box::new(file), hz))
}

impl Latency {
    pub fn new(out: Box<dyn Write + Send>, hz: Option<u16>) -> Latency {
        Latency {
            hz,
            waiting: AtomicUsize::new(0),
            inner: Mutex::new(Inner {
                seq: 0,
                pending: Vec::new(),
                scrolls: Vec::new(),
                last_scroll: Vec::new(),
                lock_wait: 0.0,
                out,
            }),
        }
    }

    /// The display link rate to use instead of the frame policy's, if fixed.
    pub fn hz(&self) -> Option<u16> {
        self.hz
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A key's bytes are about to be handed to the writer; `t0` is the
    /// event's timestamp and `key_down` when `keyDown:` got it.
    pub fn key_sent(&self, window: usize, t0: f64, key_down: f64, bytes: usize) {
        let t1 = now();
        let mut inner = self.inner();
        inner.seq += 1;
        let seq = inner.seq;
        inner.pending.push(Key {
            seq,
            window,
            bytes,
            t0,
            key_down,
            t1,
            ..Key::default()
        });
        self.waiting.fetch_add(1, Ordering::AcqRel);
    }

    /// The reader of `window` parsed output; call with the terminal locked,
    /// so a frame that sees the change also sees this.
    #[inline]
    pub fn parsed(&self, window: usize) {
        if self.waiting.load(Ordering::Acquire) == 0 {
            return;
        }
        let t = now();
        let mut inner = self.inner();
        for k in inner.pending.iter_mut().filter(|k| k.window == window && k.t2.is_none()) {
            k.t2 = Some(t);
            self.waiting.fetch_sub(1, Ordering::AcqRel);
        }
    }

    /// A frame of `window` is being built from the terminal, which is
    /// locked: takes the keys whose output it shows.
    pub fn take_for_frame(&self, window: usize, hz: u16) -> Vec<Key> {
        let mut inner = self.inner();
        if inner.pending.is_empty() {
            return Vec::new();
        }
        let t = now();
        let (mut shown, rest): (Vec<Key>, Vec<Key>) = std::mem::take(&mut inner.pending)
            .into_iter()
            .partition(|k| k.window == window && k.t2.is_some());
        inner.pending = rest;
        for k in &mut shown {
            k.frame_start = Some(t);
            k.hz = hz;
        }
        shown
    }

    /// A scroll event made at `t0` was handled in `window`.
    pub fn scrolled(&self, window: usize, t0: f64) {
        let mut inner = self.inner();
        inner.scrolls.push((window, t0));
        let t = now();
        match inner.last_scroll.iter_mut().find(|(w, _)| *w == window) {
            Some(l) => l.1 = t,
            None => inner.last_scroll.push((window, t)),
        }
    }

    /// A scroll event waited `secs` for its terminal's lock.
    pub fn lock_waited(&self, secs: f64) {
        let mut inner = self.inner();
        inner.lock_wait = inner.lock_wait.max(secs);
    }

    /// `window`'s view was scrolled in the last half second: its frames
    /// and idle ticks are logged.
    pub fn scrolling(&self, window: usize) -> bool {
        let t = now();
        self.inner().last_scroll.iter().any(|&(w, at)| w == window && t - at < 0.5)
    }

    /// A frame of `window` starts drawing while its view scrolls: the
    /// record for it, with the scroll events it takes.
    pub fn scroll_frame(&self, window: usize, hz: u16) -> ScrollFrame {
        let mut inner = self.inner();
        let mut f = ScrollFrame {
            kind: "frame",
            window,
            frame_start: now(),
            hz,
            ..ScrollFrame::default()
        };
        f.lock_wait_us = std::mem::take(&mut inner.lock_wait) * 1e6;
        let since = f.frame_start - 0.25;
        inner.scrolls.retain(|&(w, t0)| {
            if w != window {
                return true;
            }
            // Events that moved nothing (a gesture's last) stay behind
            // until the next frame: those of an earlier gesture don't
            // count for it.
            if t0 < since {
                return false;
            }
            f.events += 1;
            f.t0_first.get_or_insert(t0);
            f.t0_last = Some(t0);
            false
        });
        f
    }

    /// Append finished keys to the log.
    pub fn write(&self, keys: &[Key]) {
        self.write_all(keys, None);
    }

    fn write_all(&self, keys: &[Key], frame: Option<&ScrollFrame>) {
        let mut inner = self.inner();
        let lines = keys
            .iter()
            .map(|k| serde_json::to_string(k).expect("key serialises"))
            .chain(frame.map(|f| serde_json::to_string(f).expect("frame serialises")));
        for line in lines.collect::<Vec<_>>() {
            if let Err(e) = writeln!(inner.out, "{line}") {
                eprintln!("terminal-decline: {ENV}: {e}");
            }
        }
        let _ = inner.out.flush();
    }

    /// A tick that drew nothing while the view scrolls.
    pub fn tick(&self, window: usize, decision: String) {
        let t = Tick {
            kind: "tick",
            window,
            t: now(),
            decision,
        };
        let line = serde_json::to_string(&t).expect("tick serialises");
        let mut inner = self.inner();
        let _ = writeln!(inner.out, "{line}");
    }
}

/// The keys of one frame on their way to the screen: the GPU-completed and
/// presented handlers each fill in their time, and whichever runs last
/// writes the keys out.
pub struct FrameKeys {
    state: Mutex<(Vec<Key>, Option<ScrollFrame>, u8)>,
}

impl FrameKeys {
    pub fn new(keys: Vec<Key>) -> FrameKeys {
        FrameKeys::with_scroll(keys, None)
    }

    /// Keys and, while the view scrolls, the frame's own record.
    pub fn with_scroll(keys: Vec<Key>, frame: Option<ScrollFrame>) -> FrameKeys {
        FrameKeys {
            state: Mutex::new((keys, frame, 0)),
        }
    }

    /// The frame asked for its drawable at `asked` and had it now.
    pub fn drawable(&self, asked: f64) {
        let t = now();
        for k in &mut self.lock().0 {
            (k.built, k.drawable) = (Some(asked), Some(t));
        }
    }

    pub fn committed(&self) {
        let t = now();
        let mut st = self.lock();
        for k in &mut st.0 {
            k.commit = Some(t);
        }
        if let Some(f) = &mut st.1 {
            f.commit = Some(t);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (Vec<Key>, Option<ScrollFrame>, u8)> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn done(&self, log: &Latency, set: impl Fn(&mut Key), set_frame: impl Fn(&mut ScrollFrame)) {
        let mut st = self.lock();
        st.0.iter_mut().for_each(set);
        if let Some(f) = &mut st.1 {
            set_frame(f);
        }
        st.2 += 1;
        if st.2 == 2 {
            log.write_all(&st.0, st.1.as_ref());
        }
    }

    /// The GPU finished at `t` (host seconds).
    pub fn gpu_done(&self, log: &Latency, t: f64) {
        let t = (t > 0.0).then_some(t);
        self.done(log, |k| k.gpu_done = t, |f| f.gpu_done = t);
    }

    /// The frame was presented at `t` (0: it never was).
    pub fn presented(&self, log: &Latency, t: f64) {
        let t = (t > 0.0).then_some(t);
        self.done(log, |k| k.t3 = t, |f| f.t3 = t);
    }
}

/// A command from the harness, one line of `DECLINE_LATENCY_INPUT`.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// `key <keycode>`: post a key down and up for this virtual key code to
    /// this process through the window server (`CGEventPostToPid`).
    Key { keycode: u16 },
    /// `open <program> [args…]`: open a window running it, behind the key
    /// window.
    Open(Vec<String>),
    /// `type <hex>`: send these bytes from the first window as if typed
    /// there, without the window server (`t0` is then when the command was
    /// handled). Works without the focus, which macOS may refuse while
    /// someone types in another app.
    Type(Vec<u8>),
    /// `gesture <points> <events> <decay> <hz>`: a trackpad scroll in the
    /// first window's focused pane, played at `hz`: `events` events of
    /// `points` each (positive: back into history) with the finger down,
    /// then momentum from there, each event `decay` times the last, until
    /// under half a point. Made as scroll events with precise deltas and
    /// their phases, handled as `scrollWheel:` handles them.
    Gesture(Gesture),
    /// `wheel <lines> <events> <hz>`: a mouse wheel's clicks of `lines`
    /// each (no phases, line deltas), at `hz`.
    Wheel { lines: f64, events: u32, hz: f64 },
}

/// A trackpad scroll to play, see [`Command::Gesture`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gesture {
    pub points: f64,
    pub events: u32,
    pub decay: f64,
    pub hz: f64,
}

impl Gesture {
    /// The events: (delta in points, phase, momentum phase).
    pub fn events(&self) -> Vec<(f64, crate::scroll::Phase, crate::scroll::Phase)> {
        use crate::scroll::Phase as P;
        let mut out = Vec::new();
        for i in 0..self.events {
            out.push((self.points, if i == 0 { P::Began } else { P::Changed }, P::None));
        }
        out.push((0.0, P::Ended, P::None));
        let mut v = self.points * self.decay;
        let mut first = true;
        while v.abs() >= 0.5 && self.decay > 0.0 {
            out.push((v, P::None, if first { P::Began } else { P::Changed }));
            first = false;
            v *= self.decay;
        }
        if !first {
            out.push((0.0, P::None, P::Ended));
        }
        out
    }
}

impl Command {
    pub fn parse(line: &str) -> Option<Command> {
        let mut words = line.split_whitespace();
        let c = match words.next()? {
            "key" => Command::Key {
                keycode: words.next()?.parse().ok()?,
            },
            "open" => {
                let argv: Vec<String> = words.by_ref().map(str::to_string).collect();
                if argv.is_empty() {
                    return None;
                }
                Command::Open(argv)
            }
            "type" => {
                let hex = words.next()?;
                if hex.is_empty() || hex.len() % 2 != 0 {
                    return None;
                }
                let bytes: Option<Vec<u8>> = (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
                    .collect();
                Command::Type(bytes?)
            }
            "gesture" => {
                let g = Gesture {
                    points: words.next()?.parse().ok()?,
                    events: words.next()?.parse().ok()?,
                    decay: words.next()?.parse().ok()?,
                    hz: words.next()?.parse().ok()?,
                };
                if !(g.hz > 0.0 && (0.0..1.0).contains(&g.decay) && g.points.is_finite()) {
                    return None;
                }
                Command::Gesture(g)
            }
            "wheel" => {
                let (lines, events, hz): (f64, u32, f64) = (
                    words.next()?.parse().ok()?,
                    words.next()?.parse().ok()?,
                    words.next()?.parse().ok()?,
                );
                if hz <= 0.0 || hz.is_nan() {
                    return None;
                }
                Command::Wheel { lines, events, hz }
            }
            _ => return None,
        };
        words.next().is_none().then_some(c)
    }
}

/// With `DECLINE_LATENCY_INPUT` set (and the log on), read commands from
/// that FIFO on a thread, calling `run` on the main thread for each.
pub fn listen(run: impl Fn(Command) + Send + Sync + 'static) {
    if get().is_none() {
        return;
    }
    let Some(path) = std::env::var_os(ENV_INPUT).filter(|p| !p.is_empty()) else {
        return;
    };
    let run = std::sync::Arc::new(run);
    let spawned = std::thread::Builder::new().name("latency-input".into()).spawn(move || {
        loop {
            // Opening a FIFO waits for a writer; at its end, wait for the next.
            let Ok(f) = File::open(&path) else { return };
            for line in std::io::BufReader::new(f).lines() {
                let Ok(line) = line else { break };
                match Command::parse(&line) {
                    Some(c) => {
                        let run = run.clone();
                        dispatch2::DispatchQueue::main().exec_async(move || run(c));
                    }
                    None => eprintln!("terminal-decline: {ENV_INPUT}: unknown command {line:?}"),
                }
            }
        }
    });
    if let Err(e) = spawned {
        eprintln!("terminal-decline: {ENV_INPUT}: {e}");
    }
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreateKeyboardEvent(source: *const c_void, keycode: u16, down: bool) -> *mut c_void;
    fn CGEventPostToPid(pid: libc::pid_t, event: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

/// Post a key down and up to this process through the window server.
pub fn post_key(keycode: u16) {
    let pid = std::process::id() as libc::pid_t;
    for down in [true, false] {
        unsafe {
            let e = CGEventCreateKeyboardEvent(std::ptr::null(), keycode, down);
            if e.is_null() {
                return;
            }
            CGEventPostToPid(pid, e);
            CFRelease(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Sink {
        fn lines(&self) -> Vec<serde_json::Value> {
            let text = String::from_utf8(self.0.lock().unwrap().clone()).unwrap();
            text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
        }
    }

    #[test]
    fn follows_a_key_to_its_frame() {
        let sink = Sink::default();
        let log = Latency::new(Box::new(sink.clone()), None);
        let t0 = now();
        log.key_sent(1, t0, t0, 1);
        log.key_sent(2, t0, t0, 3);
        // Output in another window doesn't count, nor does a frame there
        // before its output.
        log.parsed(3);
        assert!(log.take_for_frame(1, 120).is_empty());
        log.parsed(1);
        // A frame of window 1 takes only window 1's key.
        let keys = log.take_for_frame(1, 120);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].seq, 1);
        let k = &keys[0];
        assert!(k.t1 >= t0 && k.t2.unwrap() >= k.t1 && k.frame_start.unwrap() >= k.t2.unwrap());
        // Window 2's key waits for its output.
        assert_eq!(log.waiting.load(Ordering::Acquire), 1);

        let frame = FrameKeys::new(keys);
        frame.committed();
        frame.presented(&log, 0.0);
        assert!(sink.lines().is_empty(), "written only once both handlers ran");
        frame.gpu_done(&log, now());
        let lines = sink.lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["seq"], 1);
        assert_eq!(lines[0]["hz"], 120);
        assert!(lines[0]["t3"].is_null(), "never presented");
        assert!(lines[0]["gpu_done"].is_f64());
    }

    #[test]
    fn clock_matches_mach_absolute_time() {
        // CACurrentMediaTime is mach_absolute_time in seconds.
        #[allow(deprecated)]
        let mach = unsafe {
            let mut tb = libc::mach_timebase_info { numer: 0, denom: 0 };
            libc::mach_timebase_info(&mut tb);
            libc::mach_absolute_time() as f64 * tb.numer as f64 / tb.denom as f64 * 1e-9
        };
        assert!((now() - mach).abs() < 0.01, "{} vs {mach}", now());
    }

    #[test]
    fn a_gesture_is_touch_then_momentum() {
        use crate::scroll::Phase as P;
        let g = Gesture {
            points: 4.0,
            events: 3,
            decay: 0.5,
            hz: 120.0,
        };
        let phases: Vec<_> = g.events().iter().map(|&(dy, p, m)| (dy, p, m)).collect();
        assert_eq!(
            phases,
            [
                (4.0, P::Began, P::None),
                (4.0, P::Changed, P::None),
                (4.0, P::Changed, P::None),
                (0.0, P::Ended, P::None),
                (2.0, P::None, P::Began),
                (1.0, P::None, P::Changed),
                (0.5, P::None, P::Changed),
                (0.0, P::None, P::Ended),
            ]
        );
    }

    #[test]
    fn parses_commands() {
        for (line, want) in [
            ("key 0", Some(Command::Key { keycode: 0 })),
            ("open yes", Some(Command::Open(vec!["yes".into()]))),
            ("open htop -d 1", Some(Command::Open(vec!["htop".into(), "-d".into(), "1".into()]))),
            ("type 617f", Some(Command::Type(vec![b'a', 0x7f]))),
            (
                "gesture 6 10 0.9 120",
                Some(Command::Gesture(Gesture {
                    points: 6.0,
                    events: 10,
                    decay: 0.9,
                    hz: 120.0,
                })),
            ),
            ("gesture 6 10 1.5 120", None),
            ("gesture 6 10 0.9 0", None),
            ("gesture 6 10 0.9", None),
            (
                "wheel -1 5 30",
                Some(Command::Wheel {
                    lines: -1.0,
                    events: 5,
                    hz: 30.0,
                }),
            ),
            ("type 6", None),
            ("type zz", None),
            ("type", None),
            ("open", None),
            ("key", None),
            ("key x", None),
            ("key 0 1", None),
            ("", None),
            ("rm -rf", None),
        ] {
            assert_eq!(Command::parse(line), want, "{line:?}");
        }
    }
}
