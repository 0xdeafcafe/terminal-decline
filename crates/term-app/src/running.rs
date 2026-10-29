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

/// The longest command line saved for a restart; a longer one isn't.
pub const MAX_COMMAND: usize = 64 * 1024;

/// The command line of the job in the foreground of `shell`'s terminal,
/// quoted for that shell, if it isn't `shell` itself: `nvim 'my file.rs'`
/// while the shell runs that, `None` at the prompt. Saved for a restart,
/// which types it at the new shell's prompt (see `reload`).
///
/// Each process's arguments are read with sysctl `KERN_PROCARGS2`. A
/// pipeline's programs (the processes of the foreground group that the
/// shell started) are joined with ` | ` in the order they started. A
/// script run through its `#!` interpreter is given as the script itself.
/// `None` too when any of them can't be read (another user's, such as
/// `sudo`'s), one is a copy of the shell (a subshell), an argument isn't
/// UTF-8, or the line would be longer than [`MAX_COMMAND`].
pub fn foreground_command(shell: libc::pid_t) -> Option<String> {
    let group = bsd_info(shell)?.e_tpgid as libc::pid_t;
    if group <= 0 || group == shell {
        return None;
    }
    let quoting = Quoting::for_shell(&name_of(shell)?);
    let own = procargs(shell);
    let own = own.as_deref().and_then(ProcArgs::parse).map(|a| a.args);
    let line = pipeline(shell, group)
        .into_iter()
        .map(|pid| {
            let buf = procargs(pid)?;
            let args = ProcArgs::parse(&buf)?;
            // A copy of the shell (a subshell, or one that hasn't started
            // its program yet) isn't a command to type.
            if own.as_ref() == Some(&args.args) {
                return None;
            }
            let argv = as_typed(args.argv()?, args.var("PATH"), cwd_of(pid).as_deref());
            Some(command_line(&argv, quoting))
        })
        .collect::<Option<Vec<_>>>()?
        .join(" | ");
    (!line.is_empty() && line.len() <= MAX_COMMAND).then_some(line)
}

/// The processes of foreground group `group` that `shell` started, in the
/// order they started; the group's leader alone if there are none.
fn pipeline(shell: libc::pid_t, group: libc::pid_t) -> Vec<libc::pid_t> {
    /// `PROC_PGRP_ONLY` in `<sys/proc_info.h>`, not in libc.
    const PROC_PGRP_ONLY: u32 = 2;
    let mut pids = vec![0 as libc::pid_t; 64];
    let size = (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
    let n = unsafe { libc::proc_listpids(PROC_PGRP_ONLY, group as u32, pids.as_mut_ptr().cast(), size) };
    pids.truncate((n.max(0) as usize) / std::mem::size_of::<libc::pid_t>());
    let mut jobs: Vec<((u64, u64, libc::pid_t), libc::pid_t)> = pids
        .into_iter()
        .filter(|&p| p > 0)
        .filter_map(|pid| {
            let info = bsd_info(pid)?;
            (info.pbi_ppid as libc::pid_t == shell).then_some(((info.pbi_start_tvsec, info.pbi_start_tvusec, pid), pid))
        })
        .collect();
    if jobs.is_empty() {
        return vec![group];
    }
    jobs.sort();
    jobs.into_iter().map(|(_, pid)| pid).collect()
}

/// A process's `KERN_PROCARGS2`: its argument count, executable path,
/// arguments and environment.
fn procargs(pid: libc::pid_t) -> Option<Vec<u8>> {
    let mut max: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 2, (&raw mut max).cast(), &mut size, std::ptr::null_mut(), 0) } != 0 || max <= 0 {
        return None;
    }
    let mut buf = vec![0u8; max as usize];
    let mut size = buf.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) } != 0 {
        return None;
    }
    buf.truncate(size);
    Some(buf)
}

/// What `KERN_PROCARGS2` gives: a native-endian `int` argument count, the
/// executable's path, NUL padding, then the arguments and the environment,
/// each string ending in a NUL.
#[derive(Debug, PartialEq)]
pub struct ProcArgs<'a> {
    pub args: Vec<&'a [u8]>,
    pub env: Vec<&'a [u8]>,
}

impl<'a> ProcArgs<'a> {
    pub fn parse(buf: &'a [u8]) -> Option<ProcArgs<'a>> {
        let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?);
        let argc = usize::try_from(argc).ok()?;
        let rest = &buf[4..];
        // The executable path, then the padding before argv[0].
        let path_end = rest.iter().position(|&b| b == 0)?;
        let start = path_end + rest[path_end..].iter().position(|&b| b != 0).unwrap_or(rest.len() - path_end);
        let mut rest = &rest[start..];
        // Each string ends in a NUL: one missing is a truncated buffer.
        let mut next = || {
            let end = rest.iter().position(|&b| b == 0)?;
            let s = &rest[..end];
            rest = &rest[end + 1..];
            Some(s)
        };
        let mut args = Vec::with_capacity(argc.min(4096));
        for _ in 0..argc {
            args.push(next()?);
        }
        let mut env = Vec::new();
        while let Some(e) = next().filter(|e| !e.is_empty()) {
            env.push(e);
        }
        Some(ProcArgs { args, env })
    }

    /// The arguments, if each is UTF-8.
    pub fn argv(&self) -> Option<Vec<String>> {
        self.args.iter().map(|a| std::str::from_utf8(a).ok().map(str::to_owned)).collect()
    }

    /// An environment variable's value.
    pub fn var(&self, name: &str) -> Option<&'a str> {
        self.env.iter().find_map(|e| {
            let value = e.strip_prefix(name.as_bytes())?.strip_prefix(b"=")?;
            std::str::from_utf8(value).ok()
        })
    }
}

fn cwd_of(pid: libc::pid_t) -> Option<std::path::PathBuf> {
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
    let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDVNODEPATHINFO, 0, (&raw mut info).cast(), size) };
    if n != size {
        return None;
    }
    let path = unsafe { std::ffi::CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr().cast()) };
    Some(std::path::PathBuf::from(std::ffi::OsStr::from_bytes(path.to_bytes())))
}

/// `argv` as it was typed: a script started through its `#!` line runs as
/// its interpreter (`/usr/bin/env node /opt/homebrew/bin/vite dev` is
/// `node /opt/homebrew/bin/vite dev` once env has run node), so the script
/// and what follows it are taken instead, and the script by its name alone
/// when that is what `path` (the program's PATH) finds. macOS 27 leaves
/// the environment out of `KERN_PROCARGS2`, so there is usually no PATH
/// and the script keeps the path the shell found it at
/// (`/opt/homebrew/bin/vite dev`), which runs the same. Relative paths are
/// from `cwd`. Anything else is left as it is.
pub fn as_typed(argv: Vec<String>, path: Option<&str>, cwd: Option<&std::path::Path>) -> Vec<String> {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    let is_script = |p: &Path| {
        let executable = std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
        let mut head = [0u8; 2];
        executable
            && std::fs::File::open(p)
                .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
                .is_ok()
            && head == *b"#!"
    };
    // An interpreter takes a few options before the script, at most.
    let found = (1..argv.len().min(4)).find_map(|i| {
        let arg = &argv[i];
        if !arg.contains('/') {
            return None;
        }
        let full = match cwd {
            Some(dir) if !arg.starts_with('/') => dir.join(arg),
            _ => Path::new(arg).to_path_buf(),
        };
        is_script(&full).then_some((i, full))
    });
    let Some((i, full)) = found else { return argv };
    let mut out = argv[i..].to_vec();
    let name = Path::new(&out[0]).file_name().map(|n| n.to_string_lossy().into_owned());
    let on_path = name.as_ref().and_then(|name| {
        let found = path?
            .split(':')
            .filter(|d| !d.is_empty())
            .map(|d| Path::new(d).join(name))
            .find(|p| is_script(p))?;
        (found.canonicalize().ok()? == full.canonicalize().ok()?).then_some(name.clone())
    });
    if let Some(name) = on_path {
        out[0] = name;
    }
    out
}

/// How the shell a command is typed into reads quotes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quoting {
    /// sh, bash, zsh and the like.
    Posix,
    /// fish: `\\` and `\'` are escapes inside single quotes, and there
    /// is no `$'…'`.
    Fish,
}

impl Quoting {
    /// For the shell named `name` (its process name, `zsh` or `fish`).
    pub fn for_shell(name: &str) -> Quoting {
        match name.trim_start_matches('-') {
            "fish" => Quoting::Fish,
            _ => Quoting::Posix,
        }
    }
}

/// `argv` as a command line for the shell: each argument quoted with
/// [`quote`], separated by spaces.
pub fn command_line<S: AsRef<str>>(argv: &[S], quoting: Quoting) -> String {
    argv.iter().map(|a| quote(a.as_ref(), quoting)).collect::<Vec<_>>().join(" ")
}

/// One argument quoted for the shell, so it reads back as itself: as it
/// is when nothing in it means anything to a shell (`foo.rs`,
/// `--model=opus`, `héllo`), else in single quotes. Control characters
/// (a newline) are escapes the shell turns back into them (`$'…'`, or for
/// fish unquoted `\n`), so the line has none: typed at a prompt, it can't
/// run before Enter is pressed. An empty argument is `''`.
pub fn quote(arg: &str, quoting: Quoting) -> String {
    if arg.is_empty() {
        return "''".into();
    }
    let plain = |(i, c): (usize, char)| {
        c.is_ascii_alphanumeric()
            || matches!(c, '/' | '.' | '_' | '-' | '+' | ',' | ':' | '@')
            // zsh expands a word starting `=cmd` to its path.
            || (c == '=' && i > 0)
            || (!c.is_ascii() && !c.is_whitespace() && !c.is_control())
    };
    if arg.char_indices().all(plain) {
        return arg.into();
    }
    match quoting {
        Quoting::Posix if arg.chars().any(char::is_control) => {
            let mut out = String::from("$'");
            for c in arg.chars() {
                match c {
                    '\\' => out.push_str("\\\\"),
                    '\'' => out.push_str("\\'"),
                    // bash's history expansion may see a `!` in `$'…'`.
                    '!' => out.push_str("\\x21"),
                    c if c.is_control() => push_escape(&mut out, c, quoting),
                    c => out.push(c),
                }
            }
            out.push('\'');
            out
        }
        Quoting::Posix => format!("'{}'", arg.replace('\'', r"'\''")),
        Quoting::Fish => {
            let mut out = String::new();
            let mut quoted = false;
            for c in arg.chars() {
                if c.is_control() {
                    if quoted {
                        out.push('\'');
                        quoted = false;
                    }
                    push_escape(&mut out, c, quoting);
                    continue;
                }
                if !quoted {
                    out.push('\'');
                    quoted = true;
                }
                match c {
                    '\\' => out.push_str("\\\\"),
                    '\'' => out.push_str("\\'"),
                    c => out.push(c),
                }
            }
            if quoted {
                out.push('\'');
            }
            out
        }
    }
}

/// A control character as a backslash escape: `\n`, `\t`, `\r`, else
/// in `$'…'` each UTF-8 byte as `\xHH` (C1 controls take two), and for
/// fish `\xHH` or (C1) `\uHHHH`.
fn push_escape(out: &mut String, c: char, quoting: Quoting) {
    match c {
        '\n' => out.push_str("\\n"),
        '\t' => out.push_str("\\t"),
        '\r' => out.push_str("\\r"),
        c if quoting == Quoting::Fish && !c.is_ascii() => out.push_str(&format!("\\u{:04x}", c as u32)),
        c => {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("\\x{b:02x}"));
            }
        }
    }
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
    /// the one expected, up to a deadline generous for a busy machine but
    /// shorter than nextest's 60 s timeout, so a failure shows what was
    /// seen.
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
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let seen = foreground_program(pty.pid());
            if want.is_none() || seen.as_deref() == want || std::time::Instant::now() >= until {
                return (pty, seen);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// What `f` gives once it is `want`, or after the same deadline as
    /// [`shell_running`]: between fork and exec, the job is still a copy
    /// of the shell, with its name (`sh`) and arguments.
    fn settled(want: Option<&str>, f: impl Fn() -> Option<String>) -> Option<String> {
        let mut got = f();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while got.as_deref() != want && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(10));
            got = f();
        }
        got
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
    fn the_command_line_of_the_job_in_the_foreground() {
        // `; :` keeps sh from exec'ing its last command, so the job stays
        // sh with these arguments (the sleep it starts isn't the shell's).
        // Not waited for by name: macOS's /bin/sh is a launcher that execs
        // bash, keeping the arguments, so the job is named `sh` only until
        // then and `bash` after.
        let (pty, _) = shell_running("set -m; /bin/sh -c 'sleep 120; :' 'a b' '' héllo \"it's\"; :", None);
        let want = r#"/bin/sh -c 'sleep 120; :' 'a b' '' héllo 'it'\''s'"#;
        assert_eq!(settled(Some(want), || foreground_command(pty.pid())).as_deref(), Some(want));
    }

    #[test]
    fn a_pipeline_is_its_programs_in_order() {
        let (pty, _) = shell_running("set -m; /bin/sleep 120 | /bin/cat -u | /usr/bin/tee; :", Some("sleep"));
        // Its later programs may not have started when the first is seen.
        let want = "/bin/sleep 120 | /bin/cat -u | /usr/bin/tee";
        assert_eq!(settled(Some(want), || foreground_command(pty.pid())).as_deref(), Some(want));
    }

    #[test]
    fn no_command_at_the_prompt() {
        let (pty, _) = shell_running("read x", None);
        assert_eq!(foreground_command(pty.pid()), None);
        assert_eq!(foreground_command(-1), None);
    }

    /// `KERN_PROCARGS2` as the kernel lays it out.
    fn procargs_buf(argc: i32, path: &[u8], pad: usize, strings: &[&[u8]]) -> Vec<u8> {
        let mut b = argc.to_ne_bytes().to_vec();
        b.extend_from_slice(path);
        b.extend(std::iter::repeat_n(0u8, pad));
        for s in strings {
            b.extend_from_slice(s);
            b.push(0);
        }
        b
    }

    #[test]
    fn parses_procargs() {
        let buf = procargs_buf(
            4,
            b"/usr/bin/nvim",
            7,
            &[
                b"nvim",
                b"",
                "my file \u{e9}.rs".as_bytes(),
                b"it's \"q\"",
                b"PATH=/a:/b",
                b"HOME=/h",
                b"",
                b"junk",
            ],
        );
        let args = ProcArgs::parse(&buf).unwrap();
        assert_eq!(args.argv().unwrap(), ["nvim", "", "my file é.rs", "it's \"q\""]);
        assert_eq!(args.env, [&b"PATH=/a:/b"[..], b"HOME=/h"]);
        assert_eq!(args.var("PATH"), Some("/a:/b"));
        assert_eq!(args.var("PAT"), None);
        assert_eq!(args.var("SHELL"), None);
        // No arguments, no environment.
        let none = procargs_buf(0, b"/bin/x", 1, &[]);
        assert_eq!(ProcArgs::parse(&none).unwrap(), ProcArgs { args: vec![], env: vec![] });
        // Not UTF-8: no command line.
        let bad = procargs_buf(2, b"/bin/x", 1, &[b"x", b"\xff\xfe"]);
        assert_eq!(ProcArgs::parse(&bad).unwrap().argv(), None);
    }

    #[test]
    fn truncated_procargs_are_refused() {
        // More arguments than there are strings.
        assert_eq!(ProcArgs::parse(&procargs_buf(3, b"/bin/x", 1, &[b"x", b"y"])), None);
        // The last argument cut off before its NUL.
        let mut cut = procargs_buf(2, b"/bin/x", 1, &[b"x", b"long"]);
        cut.truncate(cut.len() - 3);
        assert_eq!(ProcArgs::parse(&cut), None);
        assert_eq!(ProcArgs::parse(b"\x01\x00"), None);
        assert_eq!(ProcArgs::parse(&(-1i32).to_ne_bytes()), None);
        assert_eq!(ProcArgs::parse(&procargs_buf(1, b"/bin/no-nul", 0, &[])[..15]), None);
    }

    #[test]
    fn reads_a_real_process_arguments() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "read x", "a b", "", "日本 ✓"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id() as libc::pid_t;
        let buf = procargs(pid).unwrap();
        let args = ProcArgs::parse(&buf).unwrap();
        assert_eq!(args.argv().unwrap(), ["/bin/sh", "-c", "read x", "a b", "", "日本 ✓"]);
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn quotes_only_what_needs_it() {
        let q = |a: &str| quote(a, Quoting::Posix);
        for plain in [
            "nvim",
            "foo.rs",
            "/usr/bin/ssh",
            "user@host:22",
            "--model=opus",
            "a,b+c",
            "héllo",
            "日本",
            "✓",
        ] {
            assert_eq!(q(plain), plain);
        }
        assert_eq!(q(""), "''");
        assert_eq!(q("my file.rs"), "'my file.rs'");
        assert_eq!(q("it's"), r"'it'\''s'");
        assert_eq!(q("=cmd"), "'=cmd'");
        assert_eq!(q("~/x"), "'~/x'");
        assert_eq!(q("a\\b"), r"'a\b'");
        assert_eq!(q("$HOME *.rs !!"), "'$HOME *.rs !!'");
        assert_eq!(q("a\nb's!"), r"$'a\nb\'s\x21'");
        assert_eq!(q("\x1b[31m\u{85}"), r"$'\x1b[31m\xc2\x85'");
        let f = |a: &str| quote(a, Quoting::Fish);
        assert_eq!(f("my file.rs"), "'my file.rs'");
        assert_eq!(f("it's a\\b"), r"'it\'s a\\b'");
        assert_eq!(f("a\nb\t"), r"'a'\n'b'\t");
        assert_eq!(f("\u{85}x"), r"\u0085'x'");
        assert_eq!(command_line(&["nvim", "my file.rs", ""], Quoting::Posix), "nvim 'my file.rs' ''");
        assert_eq!(Quoting::for_shell("-zsh"), Quoting::Posix);
        assert_eq!(Quoting::for_shell("fish"), Quoting::Fish);
    }

    /// Arguments no simple quoting gets right everywhere.
    const AWKWARD: &[&str] = &[
        "",
        "plain",
        "two words",
        "it's",
        "\"double\"",
        "back\\slash",
        "trailing\\",
        "\\'",
        "$HOME `id` $(id)",
        "*?[a]{b,c}",
        "!! !$ ^a^b",
        "~ =x #c %self",
        "a;b|c&d<e>f",
        "héllo 日本 ✓ 🦀",
        "new\nline",
        "tab\there",
        "esc\x1b[0m",
        "c1\u{85}",
        "--model=opus",
    ];

    /// What each shell reads back from the quoted line is the arguments.
    #[test]
    fn shells_read_the_arguments_back() {
        let shells = [
            ("/bin/sh", Quoting::Posix),
            ("/bin/bash", Quoting::Posix),
            ("/bin/zsh", Quoting::Posix),
            ("/opt/homebrew/bin/bash", Quoting::Posix),
            ("/opt/homebrew/bin/fish", Quoting::Fish),
            ("/usr/local/bin/fish", Quoting::Fish),
        ];
        let mut tried = 0;
        for (shell, quoting) in shells {
            if !std::path::Path::new(shell).exists() {
                continue;
            }
            tried += 1;
            let line = command_line(AWKWARD, quoting);
            assert!(!line.chars().any(char::is_control), "{line:?}");
            let out = Command::new(shell)
                .arg("-c")
                .arg(format!("printf '%s\\000' {line}"))
                .env("HOME", std::env::temp_dir())
                .output()
                .unwrap();
            assert!(out.status.success(), "{shell}: {}", String::from_utf8_lossy(&out.stderr));
            let back: Vec<&str> = std::str::from_utf8(&out.stdout).unwrap().split_terminator('\0').collect();
            assert_eq!(back, AWKWARD, "{shell} read {line}");
        }
        assert!(tried >= 2);
    }

    #[test]
    fn a_script_is_given_as_itself() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("terminal-decline-typed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let dir = dir.canonicalize().unwrap();
        let script = dir.join("bin/tool");
        std::fs::write(&script, "#!/usr/bin/env node\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let s = |v: &[&str]| v.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        let script = script.to_str().unwrap();
        let bin = dir.join("bin");
        let path = format!("/usr/bin:{}", bin.display());
        // What node shows once env has started it: the script by name, as
        // typed, when the PATH finds it.
        assert_eq!(
            as_typed(s(&["node", script, "dev", "--port=1"]), Some(&path), None),
            ["tool", "dev", "--port=1"]
        );
        assert_eq!(as_typed(s(&["/usr/bin/env", "node", script]), Some(&path), None), ["tool"]);
        // Not on the PATH: by its path.
        assert_eq!(as_typed(s(&["node", script, "x"]), Some("/usr/bin"), None), [script, "x"]);
        // A relative path, from the program's directory.
        assert_eq!(as_typed(s(&["/bin/sh", "./bin/tool", "x"]), None, Some(&dir)), ["./bin/tool", "x"]);
        // A file that isn't a script, or a program run as it is.
        std::fs::write(dir.join("data.txt"), "#!x").unwrap();
        let data = dir.join("data.txt").display().to_string();
        assert_eq!(as_typed(s(&["cat", &data]), Some(&path), None), ["cat", data.as_str()]);
        assert_eq!(as_typed(s(&["nvim", "foo.rs"]), Some(&path), Some(&dir)), ["nvim", "foo.rs"]);
        assert_eq!(
            as_typed(s(&["/usr/bin/vim", "/etc/hosts"]), Some(&path), None),
            ["/usr/bin/vim", "/etc/hosts"]
        );
        let _ = std::fs::remove_dir_all(&dir);
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
