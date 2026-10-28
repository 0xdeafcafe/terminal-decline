//! Runs a program on a pseudo-terminal.

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

mod command;

pub use command::{Command, build_argv, build_env};

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

fn cstr(s: &str) -> io::Result<CString> {
    CString::new(s).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "nul byte in argument"))
}

impl Pty {
    pub fn spawn(cmd: &Command, size: Size) -> io::Result<Pty> {
        // Build everything the child needs before forking: after fork only
        // async-signal-safe calls are allowed.
        let shell = cmd.shell.clone().filter(|s| !s.is_empty()).unwrap_or_else(login_shell);
        let program = cstr(&shell)?;
        let argv: Vec<CString> = build_argv(&shell, &cmd.argv).iter().map(|a| cstr(a)).collect::<io::Result<_>>()?;
        let mut argv_ptrs: Vec<*const libc::c_char> = argv.iter().map(|a| a.as_ptr()).collect();
        argv_ptrs.push(std::ptr::null());

        let env: Vec<CString> = build_env(std::env::vars(), &cmd.env)
            .iter()
            .map(|(k, v)| cstr(&format!("{k}={v}")))
            .collect::<io::Result<_>>()?;
        let mut env_ptrs: Vec<*const libc::c_char> = env.iter().map(|e| e.as_ptr()).collect();
        env_ptrs.push(std::ptr::null());
        let cwd = cmd.cwd.as_deref().map(cstr).transpose()?;
        let home = std::env::var("HOME").ok().map(|h| cstr(&h)).transpose()?;

        let mut ws = size.winsize();
        let mut master: libc::c_int = -1;
        let pid = unsafe { libc::forkpty(&mut master, std::ptr::null_mut(), std::ptr::null_mut(), &mut ws) };
        if pid < 0 {
            return Err(io::Error::last_os_error());
        }
        if pid == 0 {
            unsafe {
                if let Some(dir) = cwd.as_ref().or(home.as_ref()) {
                    libc::chdir(dir.as_ptr());
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
                libc::_exit(127);
            }
        }
        let master = unsafe { OwnedFd::from_raw_fd(master) };
        unsafe {
            libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
        }
        set_nonblocking(master.as_raw_fd());
        Ok(Pty { master, pid })
    }

    pub fn pid(&self) -> libc::pid_t {
        self.pid
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
    /// new window in the same place.
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
        let pid = self.pid;
        unsafe {
            libc::kill(pid, libc::SIGHUP);
        }
        // Reap off the main thread; force it after a grace period.
        let _ = std::thread::Builder::new().name("pty-reaper".into()).spawn(move || unsafe {
            for _ in 0..20 {
                if libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) != 0 {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            libc::kill(pid, libc::SIGKILL);
            libc::waitpid(pid, std::ptr::null_mut(), 0);
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

/// Reads program output in batches. A macOS PTY hands out at most ~1 KB per
/// read, so after waiting for data this drains everything available (up to
/// `max`) before returning, letting the caller parse it under one lock.
pub struct BatchReader {
    file: File,
}

impl BatchReader {
    /// Fills `buf` with whatever is available, waiting if nothing is. Returns
    /// 0 when the program has exited.
    pub fn read_batch(&mut self, buf: &mut [u8]) -> usize {
        use std::io::Read;
        let fd = self.file.as_raw_fd();
        let mut n = 0;
        while n < buf.len() {
            match self.file.read(&mut buf[n..]) {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if n > 0 {
                        break;
                    }
                    if !wait_fd(fd, libc::POLLIN) {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                // EIO: the program and everything it started have exited.
                Err(_) => break,
            }
        }
        n
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
