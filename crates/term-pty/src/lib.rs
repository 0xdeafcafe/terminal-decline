//! Runs a program on a pseudo-terminal.

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;
use std::sync::{Arc, Mutex};

mod command;

pub use command::{Command, build_argv, build_env, exec_argv, resolve_cwd};

#[derive(Clone, Copy, Debug)]
pub struct Size {
    pub cols: u16,
    pub lines: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl Size {
    fn winsize(self) -> libc::winsize {
        libc::winsize {
            ws_row: self.lines,
            ws_col: self.cols,
            ws_xpixel: self.cols.saturating_mul(self.cell_width),
            ws_ypixel: self.lines.saturating_mul(self.cell_height),
        }
    }
}

pub struct Pty {
    master: OwnedFd,
    pid: libc::pid_t,
    child: Child,
}

/// The program's process, shared between the [`Pty`] (which hangs up on it
/// and reaps it when dropped) and whoever wants its exit status. Reaping and
/// signalling take the same lock, so a signal never reaches a process that
/// reused the pid after it was reaped.
#[derive(Clone)]
pub struct Child {
    pid: libc::pid_t,
    /// The raw `waitpid` status, once reaped.
    status: Arc<Mutex<Option<libc::c_int>>>,
}

impl Child {
    fn new(pid: libc::pid_t) -> Child {
        Child {
            pid,
            status: Arc::new(Mutex::new(None)),
        }
    }

    /// The exit status if the program has exited (reaping it), without
    /// waiting.
    pub fn try_wait(&self) -> Option<ExitStatus> {
        self.wait_with(libc::WNOHANG)
    }

    fn wait_with(&self, flags: libc::c_int) -> Option<ExitStatus> {
        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        if status.is_none() {
            let mut raw: libc::c_int = 0;
            if unsafe { libc::waitpid(self.pid, &mut raw, flags) } == self.pid {
                *status = Some(raw);
            }
        }
        status.map(ExitStatus::from_raw)
    }

    /// Send `sig` to the program (and with `group`, to its process group
    /// too), unless it has already been reaped.
    pub fn signal(&self, sig: libc::c_int, group: bool) {
        let status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        if status.is_none() {
            unsafe {
                if group {
                    libc::kill(-self.pid, sig);
                }
                libc::kill(self.pid, sig);
            }
        }
    }
}

pub fn login_shell() -> String {
    if let Ok(s) = std::env::var("SHELL")
        && !s.is_empty()
    {
        return s;
    }
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if !pw.is_null() && !(*pw).pw_shell.is_null() {
            return std::ffi::CStr::from_ptr((*pw).pw_shell).to_string_lossy().into_owned();
        }
    }
    "/bin/zsh".into()
}

/// The user's home directory from the password database, for when HOME is
/// unset.
fn passwd_home() -> Option<String> {
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_dir.is_null() {
            return None;
        }
        let dir = std::ffi::CStr::from_ptr((*pw).pw_dir).to_string_lossy().into_owned();
        (!dir.is_empty()).then_some(dir)
    }
}

fn cstr(s: &str) -> io::Result<CString> {
    CString::new(s).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "nul byte in argument"))
}

impl Pty {
    pub fn spawn(cmd: &Command, size: Size) -> io::Result<Pty> {
        // Build everything the child needs before forking: after fork only
        // async-signal-safe calls are allowed.
        let shell = cmd.shell.clone().filter(|s| !s.is_empty()).unwrap_or_else(login_shell);
        let (program, argv) = exec_argv(cmd, &shell).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        // Written to the terminal by the child if exec fails (it can't
        // format anything after forking).
        let exec_failed = format!("terminal-decline: could not start {program}\r\n");
        let program = cstr(&program)?;
        let argv: Vec<CString> = argv.iter().map(|a| cstr(a)).collect::<io::Result<_>>()?;
        let mut argv_ptrs: Vec<*const libc::c_char> = argv.iter().map(|a| a.as_ptr()).collect();
        argv_ptrs.push(std::ptr::null());

        let env = build_env(std::env::vars(), &cmd.env);
        // HOME as the child will see it, so a caller's HOME is honoured.
        let home = env
            .iter()
            .find(|(k, _)| k == "HOME")
            .map(|(_, v)| v.clone())
            .filter(|h| !h.is_empty())
            .or_else(passwd_home);
        let cwd = resolve_cwd(cmd.cwd.as_deref(), home.as_deref()).map(|d| cstr(&d)).transpose()?;
        let home = home.map(|h| cstr(&h)).transpose()?;
        let env: Vec<CString> = env.iter().map(|(k, v)| cstr(&format!("{k}={v}"))).collect::<io::Result<_>>()?;
        let mut env_ptrs: Vec<*const libc::c_char> = env.iter().map(|e| e.as_ptr()).collect();
        env_ptrs.push(std::ptr::null());

        let mut ws = size.winsize();
        let mut master: libc::c_int = -1;
        // Opening a PTY fails now and then when several processes open PTYs
        // at once (seen on macOS 27: posix_openpt fails with errno -6, a
        // negated ENXIO, about once in 400 under load), so retry a few times.
        let mut tries = 0;
        let pid = loop {
            let pid = unsafe { libc::forkpty(&mut master, std::ptr::null_mut(), std::ptr::null_mut(), &raw mut ws) };
            tries += 1;
            if pid >= 0 || tries == 5 {
                break pid;
            }
        };
        if pid < 0 {
            let e = io::Error::last_os_error();
            return Err(match e.raw_os_error() {
                Some(code) if code < 0 => io::Error::from_raw_os_error(-code),
                _ => e,
            });
        }
        if pid == 0 {
            unsafe {
                // The directory was checked before forking; if it vanished
                // since, still start in HOME rather than the app's own cwd.
                if let Some(dir) = &cwd
                    && libc::chdir(dir.as_ptr()) != 0
                    && let Some(home) = &home
                {
                    libc::chdir(home.as_ptr());
                }
                // Undo signal state inherited from the app.
                let mut set: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                libc::sigprocmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
                for sig in [
                    libc::SIGCHLD,
                    libc::SIGHUP,
                    libc::SIGINT,
                    libc::SIGQUIT,
                    libc::SIGTERM,
                    libc::SIGALRM,
                    libc::SIGPIPE,
                ] {
                    libc::signal(sig, libc::SIG_DFL);
                }
                libc::execve(program.as_ptr(), argv_ptrs.as_ptr(), env_ptrs.as_ptr());
                libc::write(libc::STDERR_FILENO, exec_failed.as_ptr().cast(), exec_failed.len());
                libc::_exit(127);
            }
        }
        let master = unsafe { OwnedFd::from_raw_fd(master) };
        unsafe {
            libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
        }
        set_nonblocking(master.as_raw_fd());
        Ok(Pty {
            master,
            pid,
            child: Child::new(pid),
        })
    }

    pub fn pid(&self) -> libc::pid_t {
        self.pid
    }

    /// A handle on the program's process, for its exit status.
    pub fn child(&self) -> Child {
        self.child.clone()
    }

    /// A handle for reading the program's output.
    pub fn reader(&self) -> io::Result<BatchReader> {
        Ok(BatchReader {
            file: File::from(self.master.try_clone()?),
        })
    }

    /// A handle for sending input to the program.
    pub fn writer(&self) -> io::Result<Writer> {
        Ok(Writer {
            file: File::from(self.master.try_clone()?),
        })
    }

    pub fn resize(&self, size: Size) -> io::Result<()> {
        let ws = size.winsize();
        if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &ws) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// The working directory of the program in the foreground, for opening a
    /// new window in the same place. macOS only; `None` elsewhere.
    #[cfg(not(target_vendor = "apple"))]
    pub fn foreground_cwd(&self) -> Option<String> {
        None
    }

    /// The working directory of the program in the foreground, for opening a
    /// new window in the same place. macOS only; `None` elsewhere.
    #[cfg(target_vendor = "apple")]
    pub fn foreground_cwd(&self) -> Option<String> {
        let pgid = unsafe { libc::tcgetpgrp(self.master.as_raw_fd()) };
        let pid = if pgid > 0 { pgid } else { self.pid };
        let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
        let n = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                (&mut info as *mut libc::proc_vnodepathinfo).cast(),
                size,
            )
        };
        if n != size {
            return None;
        }
        let path = unsafe { std::ffi::CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr().cast()) };
        Some(path.to_string_lossy().into_owned())
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        let child = self.child.clone();
        child.signal(libc::SIGHUP, false);
        // Reap off the main thread; force it after a grace period.
        let _ = std::thread::Builder::new().name("pty-reaper".into()).spawn(move || {
            for _ in 0..20 {
                if child.try_wait().is_some() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            // The child leads its own session and process group (forkpty
            // calls setsid). Kill the group too, so programs it started that
            // also ignore SIGHUP don't keep the PTY open, which would leave
            // the reader thread waiting forever.
            child.signal(libc::SIGKILL, true);
            child.wait_with(0);
        });
    }
}

/// Make a descriptor non-blocking (shared by all duplicates of the master).
fn set_nonblocking(fd: libc::c_int) {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
}

fn wait_fd(fd: libc::c_int, events: libc::c_short) -> bool {
    let mut p = libc::pollfd { fd, events, revents: 0 };
    loop {
        let r = unsafe { libc::poll(&mut p, 1, -1) };
        if r >= 0 {
            return p.revents & (libc::POLLERR | libc::POLLNVAL) == 0 || p.revents & events != 0;
        }
        if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return false;
        }
    }
}

/// Reads program output, waiting when none is available.
pub struct BatchReader {
    file: File,
}

impl BatchReader {
    /// Fills `buf` with whatever is available, waiting if nothing is. Returns
    /// 0 when the program has exited.
    pub fn read_batch(&mut self, buf: &mut [u8]) -> usize {
        use std::io::Read;
        let fd = self.file.as_raw_fd();
        // A macOS PTY returns at most ~1 KB per read and buffers ~2 KB, so
        // draining until empty costs an extra failing read per batch for
        // nothing: return what one read gives and wait only when empty.
        loop {
            match self.file.read(buf) {
                Ok(n) => return n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if !wait_fd(fd, libc::POLLIN) {
                        return 0;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                // EIO: the program and everything it started have exited.
                Err(_) => return 0,
            }
        }
    }
}

/// Writes input to the program, waiting for room when the PTY is full.
pub struct Writer {
    file: File,
}

impl Writer {
    pub fn write_all(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        use std::io::Write;
        let fd = self.file.as_raw_fd();
        while !bytes.is_empty() {
            match self.file.write(bytes) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(k) => bytes = &bytes[k..],
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if !wait_fd(fd, libc::POLLOUT) {
                        return Err(e);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}
