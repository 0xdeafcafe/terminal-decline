//! Crash handling. A panic hook writes the message, a backtrace and the
//! version to `~/Library/Logs/terminal-decline/crash-<time>.log` and logs a
//! fault to os_log (subsystem `red.forbes.terminal-decline`), then lets the
//! panic go on.
//!
//! What happens next depends on the thread. A panic while a session's
//! reader thread parses output is caught there (see [`crate::session`]): that
//! window shows an error and the others keep running. A panic while the
//! main thread builds one pane's frame is caught where that pane is built
//! ([`contain`], splits design §7): that pane shows an error and the others
//! keep drawing. Any other panic on the main thread (AppKit) aborts once it
//! is logged, since AppKit's state can't be trusted after it.

use std::backtrace::Backtrace;
use std::cell::RefCell;
use std::fs::OpenOptions;
use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::{Once, OnceLock};

/// Where crash logs go: set once, by [`install`].
static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

thread_local! {
    /// The crash log written for this thread's last panic.
    static LAST_LOG: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    /// Inside [`contain`]: a panic here is caught, so the main thread
    /// doesn't abort for it.
    static CONTAINED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Run `f`, catching a panic in it (logged as any other) instead of
/// aborting on the main thread. Only for code whose state is thrown away
/// or replaced after a panic, as a pane's frame build is.
pub fn contain<R>(f: impl FnOnce() -> R) -> std::thread::Result<R> {
    struct Leave;
    impl Drop for Leave {
        fn drop(&mut self) {
            CONTAINED.set(CONTAINED.get() - 1);
        }
    }
    CONTAINED.set(CONTAINED.get() + 1);
    let _leave = Leave;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
}

/// `~/Library/Logs/terminal-decline`.
pub fn logs_dir(home: &Path) -> PathBuf {
    home.join("Library/Logs/terminal-decline")
}

/// Install the panic hook, writing logs into `dir` (created when needed).
/// With `abort_on_main`, a panic on the main thread aborts once logged.
/// Only the first call does anything.
pub fn install(dir: PathBuf, abort_on_main: bool) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = LOG_DIR.set(dir);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let report = report(info, &Backtrace::force_capture());
            let path = LOG_DIR.get().and_then(|d| write_log(d, &report).ok());
            let summary = match &path {
                Some(p) => format!("{}; log: {}", first_lines(&report), p.display()),
                None => first_lines(&report),
            };
            oslog::fault(&summary);
            LAST_LOG.with(|l| *l.borrow_mut() = path);
            previous(info);
            if abort_on_main && CONTAINED.get() == 0 && unsafe { libc::pthread_main_np() } != 0 {
                std::process::abort();
            }
        }));
    });
}

/// The crash log written for the last panic on this thread, if any.
pub fn take_last_log() -> Option<PathBuf> {
    LAST_LOG.with(|l| l.borrow_mut().take())
}

/// `path` with the home directory shown as `~`.
pub fn tilde(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty())
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

fn report(info: &PanicHookInfo, backtrace: &Backtrace) -> String {
    let thread = std::thread::current();
    format!(
        "Terminal Decline {} hit a bug\nthread '{}' {info}\n\nbacktrace:\n{backtrace}\n",
        env!("CARGO_PKG_VERSION"),
        thread.name().unwrap_or("<unnamed>"),
    )
}

/// The version, thread and panic message: the report up to the backtrace.
fn first_lines(report: &str) -> String {
    report.split("\n\nbacktrace:").next().unwrap_or(report).replace('\n', " ")
}

/// Write `report` to a new `crash-<local time>.log` in `dir`.
fn write_log(dir: &Path, report: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let stamp = local_time();
    for n in 1.. {
        let name = if n == 1 {
            format!("crash-{stamp}.log")
        } else {
            format!("crash-{stamp}-{n}.log")
        };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                f.write_all(report.as_bytes())?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 100 => {}
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

/// `2026-10-28-153012` in local time.
fn local_time() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    format!(
        "{:04}-{:02}-{:02}-{:02}{:02}{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// The few lines of FFI needed for `os_log_with_type(log, OS_LOG_TYPE_FAULT,
/// "%{public}s", msg)`, which is a C macro around `_os_log_impl`.
mod oslog {
    use std::ffi::{CString, c_char, c_void};
    use std::sync::OnceLock;

    const OS_LOG_TYPE_FAULT: u8 = 0x11;

    unsafe extern "C" {
        /// This image's Mach-O header; os_log finds the format string from it.
        static __dso_handle: u8;
        fn os_log_create(subsystem: *const c_char, category: *const c_char) -> *mut c_void;
        fn _os_log_impl(dso: *const c_void, log: *mut c_void, kind: u8, format: *const c_char, buf: *const u8, size: u32);
    }

    /// The format string must live in this section, as the C macro puts it,
    /// for `log show` to decode the message.
    #[unsafe(link_section = "__TEXT,__oslogstring,cstring_literals")]
    static FORMAT: [u8; 11] = *b"%{public}s\0";

    struct Handle(*mut c_void);
    // os_log_t is thread-safe and lives forever.
    unsafe impl Send for Handle {}
    unsafe impl Sync for Handle {}

    pub fn fault(msg: &str) {
        static LOG: OnceLock<Handle> = OnceLock::new();
        let log = LOG.get_or_init(|| Handle(unsafe { os_log_create(c"red.forbes.terminal-decline".as_ptr(), c"crash".as_ptr()) }));
        if log.0.is_null() {
            return;
        }
        let Ok(msg) = CString::new(msg.replace('\0', " ")) else { return };
        // The argument buffer the macro builds: summary (has non-scalar
        // arguments), argument count, then per argument a descriptor (string,
        // public), its size and its value.
        let mut buf = [0u8; 12];
        buf[0] = 0x02;
        buf[1] = 1;
        buf[2] = 0x22;
        buf[3] = 8;
        buf[4..].copy_from_slice(&(msg.as_ptr() as u64).to_le_bytes());
        unsafe {
            _os_log_impl(
                (&raw const __dso_handle).cast(),
                log.0,
                OS_LOG_TYPE_FAULT,
                FORMAT.as_ptr().cast(),
                buf.as_ptr(),
                buf.len() as u32,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn writes_a_new_log_each_time() {
        let dir = TempDir::new("crash-logs");
        let a = write_log(&dir.join("logs"), "one").unwrap();
        let b = write_log(&dir.join("logs"), "two").unwrap();
        assert_ne!(a, b);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "one");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "two");
        let name = a.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("crash-20") && name.ends_with(".log"), "{name}");
    }

    #[test]
    fn summary_is_the_report_before_the_backtrace() {
        let r = "Terminal Decline 1 hit a bug\nthread 'x' panicked at a.rs:1:2:\nboom\n\nbacktrace:\n  0: f\n";
        assert_eq!(first_lines(r), "Terminal Decline 1 hit a bug thread 'x' panicked at a.rs:1:2: boom");
    }

    #[test]
    fn contain_catches_and_leaves() {
        assert_eq!(contain(|| 3).ok(), Some(3));
        let caught = contain(|| {
            assert_eq!(CONTAINED.get(), 1);
            panic!("contained in a unit test, ignore")
        });
        assert!(caught.is_err());
        assert_eq!(CONTAINED.get(), 0);
    }

    #[test]
    fn os_log_does_not_crash() {
        oslog::fault("terminal-decline unit test: os_log fault, ignore");
    }
}
