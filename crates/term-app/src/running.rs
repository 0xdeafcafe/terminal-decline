//! Whether a window is running a program, for asking before it is closed
//! (`confirm_close = true`), and what the question says.

use std::os::unix::ffi::OsStrExt;

/// The name of the program in the foreground of `shell`'s terminal, if it
/// isn't `shell` itself: `vim` while the shell runs vim, `None` at the
/// prompt. A window started with `-e PROGRAM` runs that program as its
/// "shell", so it counts only while it starts another in the foreground.
///
/// Read from the shell's `proc_bsdinfo`: `e_tpgid` is the terminal's
/// foreground process group, whose leader is the job the shell started.
pub fn foreground_program(shell: libc::pid_t) -> Option<String> {
    let group = bsd_info(shell)?.e_tpgid as libc::pid_t;
    if group <= 0 || group == shell {
        return None;
    }
    Some(name_of(group).unwrap_or_else(|| format!("process {group}")))
}

/// Whether a process is running the executable at `exe` (a pin's, before
/// removing it). Compared by canonical path.
pub fn executable_running(exe: &std::path::Path) -> bool {
    let Ok(exe) = exe.canonicalize() else { return false };
    let n = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if n <= 0 {
        return false;
    }
    // Room for processes started meanwhile.
    let mut pids = vec![0 as libc::pid_t; n as usize + 64];
    let size = (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
    let n = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), size) };
    pids.truncate(n.max(0) as usize);
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    pids.into_iter().filter(|&p| p > 0).any(|pid| {
        let len = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
        len > 0 && std::path::Path::new(std::ffi::OsStr::from_bytes(&buf[..len as usize])) == exe
    })
}

fn bsd_info(pid: libc::pid_t) -> Option<libc::proc_bsdinfo> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    (n == size).then_some(info)
}

fn name_of(pid: libc::pid_t) -> Option<String> {
    let mut buf = [0u8; 256];
    let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

/// The title and text of the question asked before closing a window
/// running `program`.
pub fn close_question(program: &str) -> (String, String) {
    (
        "Close this window?".into(),
        format!("{program} is still running in it. Closing the window ends it."),
    )
}

/// The title and text of the question asked before quitting while
/// `programs` run, one per window.
pub fn quit_question(app_name: &str, programs: &[String]) -> (String, String) {
    let mut names: Vec<&str> = Vec::new();
    for p in programs {
        if !names.contains(&p.as_str()) {
            names.push(p);
        }
    }
    let text = match programs.len() {
        1 => format!("{} is still running. Quitting ends it.", programs[0]),
        n => format!(
            "Programs are still running in {n} windows ({}). Quitting ends them.",
            names.join(", ")
        ),
    };
    (format!("Quit {app_name}?"), text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    /// A shell on a PTY, as a window runs one, and the foreground program
    /// once it is `want`. With `want` as `None` the shell is only looked at
    /// once, for a script that never starts a job. A job's process group
    /// is in the foreground from the fork, before it execs the program, so
    /// waiting for any program would see the shell's own name; wait for
    /// the one expected, up to a generous deadline for a busy machine.
    fn shell_running(script: &str, want: Option<&str>) -> (term_pty::Pty, Option<String>) {
        let cmd = term_pty::Command {
            argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
            cwd: None,
            env: Vec::new(),
            shell: Some("/bin/sh".into()),
            direct: false,
        };
        let size = term_pty::Size {
            cols: 80,
            lines: 24,
            cell_width: 8,
            cell_height: 16,
        };
        let pty = term_pty::Pty::spawn(&cmd, size).unwrap();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let seen = foreground_program(pty.pid());
            if want.is_none() || seen.as_deref() == want || std::time::Instant::now() >= until {
                return (pty, seen);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn sees_the_job_in_the_foreground() {
        // With job control on, the shell gives `sleep` its own process
        // group and the terminal to it, as an interactive shell does.
        let (_pty, fg) = shell_running("set -m; /bin/sleep 120; :", Some("sleep"));
        assert_eq!(fg.as_deref(), Some("sleep"));
    }

    #[test]
    fn a_shell_at_its_prompt_runs_nothing() {
        let (_pty, fg) = shell_running("read x", None);
        assert_eq!(fg, None);
        assert_eq!(foreground_program(-1), None);
    }

    #[test]
    fn names_a_process() {
        let mut child = Command::new("/bin/sleep").arg("5").stdout(Stdio::null()).spawn().unwrap();
        let name = name_of(child.id() as libc::pid_t);
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(name.as_deref(), Some("sleep"));
    }

    #[test]
    fn questions_name_the_programs() {
        assert_eq!(
            close_question("vim"),
            (
                "Close this window?".into(),
                "vim is still running in it. Closing the window ends it.".into()
            )
        );
        assert_eq!(
            quit_question("Terminal Decline", &["top".into()]),
            ("Quit Terminal Decline?".into(), "top is still running. Quitting ends it.".into())
        );
        assert_eq!(
            quit_question("X", &["vim".into(), "vim".into(), "top".into()]).1,
            "Programs are still running in 3 windows (vim, top). Quitting ends them."
        );
    }
}
