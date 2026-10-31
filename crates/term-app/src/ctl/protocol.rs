//! The agent API's wire format (docs/design/agent-api.md §4): one JSON
//! request per line, one JSON reply per line, over a Unix socket only the
//! user can reach.

use std::collections::BTreeMap;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Moves the socket's directory (the tests use a directory of their own).
pub const DIR_ENV: &str = "TERMINAL_DECLINE_CTL_DIR";
/// How long the headless server stays once nothing is running, in ms.
pub const IDLE_ENV: &str = "TERMINAL_DECLINE_CTL_IDLE_MS";

/// Exit codes of `terminal-decline ctl`.
pub mod exit {
    /// Done; for `wait-for`, matched.
    pub const OK: i32 = 0;
    /// The server refused or failed the request.
    pub const FAILED: i32 = 1;
    /// Bad arguments.
    pub const USAGE: i32 = 2;
    /// `wait-for` timed out.
    pub const TIMEOUT: i32 = 3;
    /// No server could be reached or started.
    pub const NO_SERVER: i32 = 4;
    /// `wait-for --text/--regex`: the program exited without it showing.
    pub const EXITED: i32 = 5;
}

/// One request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    /// Who is serving: `app` or `headless`, its pid and version.
    Hello,
    Spawn(Spawn),
    Send {
        session: String,
        actions: Vec<Action>,
    },
    Screen {
        session: String,
        /// Each cell's text, colours and attributes too.
        #[serde(default)]
        json: bool,
        /// History lines before the screen, at most.
        #[serde(default)]
        scrollback: usize,
    },
    WaitFor(WaitFor),
    Resize {
        session: String,
        cols: u16,
        rows: u16,
    },
    Screenshot {
        session: String,
        /// Absolute path of the PNG to write.
        path: String,
    },
    Stats {
        session: String,
    },
    /// Record to `path` as asciicast v2 from now, or stop (no path).
    Record {
        session: String,
        #[serde(default)]
        path: Option<String>,
        /// Input events (`"i"`) too.
        #[serde(default)]
        input: bool,
    },
    List,
    /// Hang up on the program and forget the session, or with `signal`
    /// only send it that signal.
    Kill {
        session: String,
        #[serde(default)]
        signal: Option<String>,
    },
}

/// `spawn`'s settings. Unset ones are the config's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Spawn {
    /// The session's id; the server picks `1`, `2`, … without one.
    pub name: Option<String>,
    /// Program and arguments; empty is the login shell.
    pub argv: Vec<String>,
    pub cols: u16,
    pub rows: u16,
    pub cwd: Option<String>,
    /// On top of the server's environment (the client sends its own).
    pub env: BTreeMap<String, String>,
    /// TERM, instead of `xterm-256color`.
    pub term: Option<String>,
    /// Run the program through the login shell, as a window does,
    /// rather than exec'ing it directly (found on the env's PATH).
    pub shell: bool,
    /// Show it in a window of the app, in the background; closing the
    /// window kills the session. Refused by a headless server.
    pub window: bool,
    pub font: Option<String>,
    pub font_size: Option<f64>,
    pub theme: Option<String>,
    /// `light` or `dark`; unset, the config's `appearance` (the
    /// system's).
    pub appearance: Option<String>,
    /// Device pixels per point for screenshots and cell pixel sizes.
    pub scale: Option<f64>,
    pub scrollback: Option<usize>,
    /// Record from the start to this asciicast file.
    pub record: Option<String>,
}

impl Default for Spawn {
    fn default() -> Spawn {
        Spawn {
            name: None,
            argv: Vec::new(),
            cols: 80,
            rows: 24,
            cwd: None,
            env: BTreeMap::new(),
            term: None,
            shell: false,
            window: false,
            font: None,
            font_size: None,
            theme: None,
            appearance: None,
            scale: None,
            scrollback: None,
            record: None,
        }
    }
}

/// One thing `send` does, in order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// Typed key by key.
    Text(String),
    /// A key by name with modifiers: `ctrl+c`, `shift+tab`, `alt+left`,
    /// `f5`, `enter`.
    Key(String),
    /// Pasted: bracketed when the program asked.
    Paste(String),
    /// Bytes as they are (escapes `\e`, `\r`, `\n`, `\t`, `\\`, `\xHH`).
    Raw(String),
    Mouse(Mouse),
    /// Wait this many ms before the next action.
    Sleep(u64),
}

/// A mouse event at a cell (0-based line and column of the screen).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mouse {
    /// `click` (press and release), `press`, `release`, `move`, `scroll`.
    pub kind: String,
    pub line: usize,
    pub col: usize,
    /// `left` (default), `middle`, `right`.
    #[serde(default)]
    pub button: Option<String>,
    /// `shift`, `alt`, `ctrl` joined by `+`.
    #[serde(default)]
    pub mods: Option<String>,
    /// `scroll`: lines, positive up (back), negative down.
    #[serde(default)]
    pub lines: Option<i32>,
}

/// `wait-for`: exactly one of `text`, `regex`, `idle_ms`, `exit`,
/// `command`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WaitFor {
    pub session: String,
    pub text: Option<String>,
    pub regex: Option<String>,
    /// The output has been quiet this long.
    pub idle_ms: Option<u64>,
    pub exit: bool,
    /// The next command the shell finishes (OSC 133 marks, `--shell`
    /// sessions): its exit status and duration come back.
    pub command: bool,
    pub timeout_ms: Option<u64>,
}

/// `wait-for`'s timeout when none is given.
pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;

/// Where the socket lives: [`DIR_ENV`], else
/// `$TMPDIR/terminal-decline-$UID`.
pub fn dir() -> PathBuf {
    if let Some(d) = std::env::var_os(DIR_ENV).filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let tmp = std::env::var_os("TMPDIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp".into());
    tmp.join(format!("terminal-decline-{}", unsafe { libc::getuid() }))
}

pub fn socket_path(dir: &Path) -> PathBuf {
    dir.join("ctl.sock")
}

/// Make `dir` (0700) if it isn't there, and check that only this user
/// can reach it: owned by us, not a symlink, no access for others.
pub fn ensure_dir(dir: &Path) -> io::Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    check_dir(dir)
}

/// [`ensure_dir`]'s check on its own.
pub fn check_dir(dir: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(dir)?;
    let refuse = |why: &str| Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{}: {why}", dir.display())));
    if !meta.is_dir() {
        return refuse("not a directory");
    }
    if meta.uid() != unsafe { libc::geteuid() } {
        return refuse("belongs to another user");
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return refuse("others can reach it (it must be 0700)");
    }
    Ok(())
}

/// Make the socket file only ours (0600).
pub fn restrict_socket(path: &Path) -> io::Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

/// Whether the other end of `stream` runs as this user.
pub fn same_user(stream: &UnixStream) -> bool {
    peer_uid(stream) == Some(unsafe { libc::geteuid() })
}

/// The other end's effective uid (`getpeereid`).
pub fn peer_uid(stream: &UnixStream) -> Option<libc::uid_t> {
    let (mut uid, mut gid) = (0, 0);
    let r = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    (r == 0).then_some(uid)
}

/// `\e`, `\r`, `\n`, `\t`, `\\` and `\xHH` as the bytes they stand for.
pub fn unescape(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let Some(&c) = b.get(i + 1) else {
            return Err("a lone \\ at the end".into());
        };
        i += 2;
        out.push(match c {
            b'e' => 0x1b,
            b'r' => b'\r',
            b'n' => b'\n',
            b't' => b'\t',
            b'\\' => b'\\',
            b'x' => {
                let hex = s.get(i..i + 2).ok_or("\\x needs two hex digits")?;
                i += 2;
                u8::from_str_radix(hex, 16).map_err(|_| format!("\\x{hex}: not hex"))?
            }
            other => return Err(format!("unknown escape \\{}", other as char)),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(r: &Request) {
        let line = serde_json::to_string(r).unwrap();
        assert!(!line.contains('\n'), "{line}");
        let back: Request = serde_json::from_str(&line).unwrap();
        assert_eq!(&back, r, "{line}");
    }

    #[test]
    fn requests_round_trip() {
        for r in [
            Request::Hello,
            Request::List,
            Request::Spawn(Spawn {
                name: Some("top".into()),
                argv: vec!["htop".into(), "-d".into(), "10".into()],
                cols: 120,
                rows: 40,
                env: BTreeMap::from([("A".into(), "b c".into())]),
                theme: Some("Dracula".into()),
                scale: Some(1.0),
                ..Spawn::default()
            }),
            Request::Send {
                session: "1".into(),
                actions: vec![
                    Action::Text("ls\n".into()),
                    Action::Key("ctrl+c".into()),
                    Action::Paste("a\nb".into()),
                    Action::Raw("\\e[A".into()),
                    Action::Sleep(20),
                    Action::Mouse(Mouse {
                        kind: "click".into(),
                        line: 3,
                        col: 4,
                        button: Some("right".into()),
                        mods: None,
                        lines: None,
                    }),
                ],
            },
            Request::Screen {
                session: "1".into(),
                json: true,
                scrollback: 100,
            },
            Request::WaitFor(WaitFor {
                session: "1".into(),
                regex: Some("^\\$ ".into()),
                timeout_ms: Some(500),
                ..WaitFor::default()
            }),
            Request::Resize {
                session: "x".into(),
                cols: 10,
                rows: 5,
            },
            Request::Screenshot {
                session: "x".into(),
                path: "/tmp/a.png".into(),
            },
            Request::Stats { session: "x".into() },
            Request::Record {
                session: "x".into(),
                path: None,
                input: false,
            },
            Request::Kill {
                session: "x".into(),
                signal: Some("INT".into()),
            },
        ] {
            round_trip(&r);
        }
    }

    #[test]
    fn requests_read_with_defaults() {
        let r: Request = serde_json::from_str(r#"{"cmd":"spawn","argv":["vim"]}"#).unwrap();
        let Request::Spawn(s) = r else { panic!("{r:?}") };
        assert_eq!((s.cols, s.rows, s.argv.as_slice()), (80, 24, &["vim".to_string()][..]));
        let r: Request = serde_json::from_str(r#"{"cmd":"send","session":"1","actions":[{"key":"enter"},{"text":"q"}]}"#).unwrap();
        assert_eq!(
            r,
            Request::Send {
                session: "1".into(),
                actions: vec![Action::Key("enter".into()), Action::Text("q".into())]
            }
        );
        let r: Request = serde_json::from_str(r#"{"cmd":"wait-for","session":"1","exit":true}"#).unwrap();
        assert_eq!(
            r,
            Request::WaitFor(WaitFor {
                session: "1".into(),
                exit: true,
                ..WaitFor::default()
            })
        );
        for bad in [
            r#"{"cmd":"launch-missiles"}"#,
            r#"{"cmd":"send"}"#,
            "not json",
            r#"{"cmd":"resize","session":"1","cols":-1,"rows":2}"#,
        ] {
            assert!(serde_json::from_str::<Request>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn unescapes() {
        assert_eq!(unescape("a\\e[A\\r\\n\\t\\\\\\x7f").unwrap(), b"a\x1b[A\r\n\t\\\x7f");
        for bad in ["\\", "\\q", "\\x", "\\xzz"] {
            assert!(unescape(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_directory_must_be_ours_alone() {
        let base = crate::testutil::TempDir::new("ctl-dir");
        let dir = base.join("d");
        ensure_dir(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        // Opened up: refused, and not fixed behind the user's back.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(ensure_dir(&dir).is_err());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        ensure_dir(&dir).unwrap();
        // A symlink to a good directory is refused too.
        let link = base.join("l");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(ensure_dir(&link).is_err());
        let file = base.join("f");
        std::fs::write(&file, "").unwrap();
        assert!(ensure_dir(&file).is_err());
    }

    #[test]
    fn peers_are_this_user() {
        let (a, b) = UnixStream::pair().unwrap();
        assert!(same_user(&a) && same_user(&b));
        assert_eq!(peer_uid(&a), Some(unsafe { libc::geteuid() }));
    }
}
