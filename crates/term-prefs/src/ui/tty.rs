//! The terminal the TUI runs in: raw mode through termios, the modes it
//! turns on (alternate screen, SGR mouse, bracketed paste, kitty flag 1)
//! and turns off again, and a blocking wait for input, a resize, a hangup
//! or a file change. No timers: the wait has a timeout only while a write
//! is held back.

use std::io::{self, Write as _};
use std::os::fd::RawFd;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

/// Turned on at start: alternate screen, button mouse in SGR form,
/// bracketed paste, kitty keyboard flag 1 (disambiguate), cleared.
pub const ENTER: &str = "\x1b[?1049h\x1b[?1000h\x1b[?1006h\x1b[?2004h\x1b[>1u\x1b[H\x1b[2J";
/// Turned off again, in reverse.
pub const LEAVE: &str = "\x1b[<u\x1b[?2004l\x1b[?1006l\x1b[?1000l\x1b[0m\x1b[?25h\x1b[?1049l";

static SAVED: Mutex<Option<libc::termios>> = Mutex::new(None);
/// The write end of the wake pipe, for signal handlers and file watches.
static WAKE_FD: AtomicI32 = AtomicI32::new(-1);

/// Why the wait returned.
#[derive(Debug, PartialEq, Eq)]
pub enum Wake {
    Input(Vec<u8>),
    Resize,
    /// SIGHUP or SIGTERM: the tab is closing.
    Hangup,
    /// A watched file changed.
    Changed,
    Timeout,
    /// Input ended.
    Eof,
}

const RESIZE: u8 = b'W';
const HANGUP: u8 = b'H';
const CHANGED: u8 = b'C';

/// Raw mode and the wake pipe, restored on drop.
pub struct Tty {
    input: RawFd,
    wake_read: RawFd,
}

extern "C" fn on_signal(sig: libc::c_int) {
    let byte = if sig == libc::SIGWINCH { RESIZE } else { HANGUP };
    poke(byte);
}

fn poke(byte: u8) {
    let fd = WAKE_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        // SAFETY: write(2) is async-signal-safe; a full pipe drops the byte,
        // which is fine as one pending byte already wakes the loop.
        unsafe {
            libc::write(fd, (&raw const byte).cast(), 1);
        }
    }
}

/// Wakes the wait to say a watched file changed (any thread).
pub fn file_changed() {
    poke(CHANGED);
}

/// Puts the terminal back as it was: leaves the modes and raw mode. Safe
/// to call twice; used by the panic hook too.
pub fn restore() {
    let _ = io::stdout().write_all(LEAVE.as_bytes());
    let _ = io::stdout().flush();
    if let Some(t) = SAVED.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        // SAFETY: restoring settings read from the same fd.
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, t);
        }
    }
}

impl Tty {
    /// Raw mode on stdin, the modes on, signals routed to the wait.
    pub fn open() -> io::Result<Tty> {
        let input = libc::STDIN_FILENO;
        // SAFETY: plain libc calls on our own fds and a zeroed termios.
        unsafe {
            if libc::isatty(input) != 1 {
                return Err(io::Error::other("not a terminal: run it in one"));
            }
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(input, &mut t) != 0 {
                return Err(io::Error::last_os_error());
            }
            *SAVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(t);
            let mut raw = t;
            raw.c_iflag &= !(libc::BRKINT | libc::ICRNL | libc::INPCK | libc::ISTRIP | libc::IXON);
            raw.c_oflag &= !libc::OPOST;
            raw.c_cflag |= libc::CS8;
            raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG);
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            if libc::tcsetattr(input, libc::TCSAFLUSH, &raw) != 0 {
                return Err(io::Error::last_os_error());
            }
            let mut fds = [0; 2];
            if libc::pipe(fds.as_mut_ptr()) != 0 {
                return Err(io::Error::last_os_error());
            }
            for fd in fds {
                libc::fcntl(fd, libc::F_SETFL, libc::fcntl(fd, libc::F_GETFL) | libc::O_NONBLOCK);
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            WAKE_FD.store(fds[1], Ordering::Relaxed);
            let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
            for sig in [libc::SIGWINCH, libc::SIGHUP, libc::SIGTERM] {
                libc::signal(sig, handler);
            }
            let mut out = io::stdout();
            out.write_all(ENTER.as_bytes())?;
            out.flush()?;
            Ok(Tty { input, wake_read: fds[0] })
        }
    }

    /// Leaves raw mode and the modes, for `$EDITOR`.
    pub fn suspend(&self) {
        restore();
    }

    /// Back after `$EDITOR`.
    pub fn resume(&self) -> io::Result<()> {
        if let Some(t) = *SAVED.lock().unwrap_or_else(|e| e.into_inner()) {
            let mut raw = t;
            raw.c_iflag &= !(libc::BRKINT | libc::ICRNL | libc::INPCK | libc::ISTRIP | libc::IXON);
            raw.c_oflag &= !libc::OPOST;
            raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG);
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            // SAFETY: as in `open`.
            unsafe {
                libc::tcsetattr(self.input, libc::TCSAFLUSH, &raw);
            }
        }
        let mut out = io::stdout();
        out.write_all(ENTER.as_bytes())?;
        out.flush()
    }

    /// The window size in cells.
    pub fn size(&self) -> (usize, usize) {
        // SAFETY: TIOCGWINSZ fills a winsize.
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
                (ws.ws_col as usize, ws.ws_row as usize)
            } else {
                (80, 24)
            }
        }
    }

    pub fn write(&self, s: &str) -> io::Result<()> {
        let mut out = io::stdout().lock();
        out.write_all(s.as_bytes())?;
        out.flush()
    }

    /// Blocks until input, a signal or a file change; with `timeout`, at
    /// most that long.
    pub fn wait(&self, timeout: Option<Duration>) -> io::Result<Wake> {
        let mut fds = [
            libc::pollfd {
                fd: self.wake_read,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: self.input,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let ms = timeout.map_or(-1, |t| t.as_millis().min(i32::MAX as u128) as i32);
        // SAFETY: two valid pollfds.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, ms) };
        if n < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::Interrupted {
                Ok(Wake::Timeout)
            } else {
                Err(e)
            };
        }
        if n == 0 {
            return Ok(Wake::Timeout);
        }
        if fds[0].revents & libc::POLLIN != 0 {
            let mut buf = [0u8; 64];
            // SAFETY: reading into our buffer.
            let got = unsafe { libc::read(self.wake_read, buf.as_mut_ptr().cast(), buf.len()) };
            let got = &buf[..got.max(0) as usize];
            if got.contains(&HANGUP) {
                return Ok(Wake::Hangup);
            }
            if got.contains(&RESIZE) {
                return Ok(Wake::Resize);
            }
            if got.contains(&CHANGED) {
                return Ok(Wake::Changed);
            }
        }
        if fds[1].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            let mut buf = [0u8; 4096];
            // SAFETY: reading into our buffer.
            let got = unsafe { libc::read(self.input, buf.as_mut_ptr().cast(), buf.len()) };
            if got <= 0 {
                return Ok(Wake::Eof);
            }
            return Ok(Wake::Input(buf[..got as usize].to_vec()));
        }
        Ok(Wake::Timeout)
    }
}

impl Drop for Tty {
    fn drop(&mut self) {
        restore();
        WAKE_FD.store(-1, Ordering::Relaxed);
        // SAFETY: closing our own pipe.
        unsafe {
            libc::close(self.wake_read);
        }
    }
}
