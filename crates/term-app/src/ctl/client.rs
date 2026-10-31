//! `terminal-decline ctl …`: arguments to a request, the request to the
//! server (started when there is none), the reply printed.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::protocol::{self, Action, Mouse, Request, Spawn, WaitFor, exit};

pub const HELP: &str = "\
usage: terminal-decline ctl COMMAND [ARGS...]

Drive terminals from agents and tests (docs/design/agent-api.md). Each
command prints one line of JSON (`screen` prints text unless --json).
Sessions are headless: no windows. Served by the running app, or by a
headless server started on demand that exits when idle.

  socket                        print the socket's path
  hello                         who serves (app or headless)
  spawn [OPTIONS] [-- PROGRAM [ARGS...]]
        --name NAME             the session's id (default 1, 2, ...)
        --size COLSxROWS        default 80x24
        --cwd DIR               default the current directory
        --env KEY=VALUE         more environment (yours is passed on)
        --term TERM             default xterm-256color
        --shell                 run through your login shell, as a window does
        --font FAMILY --font-size PT --theme NAME --appearance light|dark
        --scale N               pixels per point for screenshots (default 2)
        --scrollback LINES      --record FILE.cast
        --window                (not yet) show it in a window
  send ID ACTION...             in order:
        --text TEXT             typed key by key (\\n is Enter)
        --key KEY               ctrl+c, enter, shift+tab, alt+left, f5, ...
        --paste TEXT            pasted (bracketed when the program asked)
        --raw BYTES             as they are (\\e \\r \\n \\t \\\\ \\xHH)
        --mouse KIND:LINE,COL[:BUTTON|:LINES]   click, press, release,
                                move, scroll (0-based cells)
        --sleep MS
  screen ID [--json] [--scrollback N]
  wait-for ID (--text T | --regex RE | --idle MS | --exit | --command)
        [--timeout MS]   --command: the next command a --shell session
                         finishes; its status and duration come back
  resize ID COLSxROWS
  screenshot ID FILE.png
  stats ID
  record ID FILE.cast [--input] | record ID --stop
  list
  kill ID [--signal SIG]

Exit status: 0 done (wait-for: matched), 1 the request failed, 2 bad
arguments, 3 wait-for timed out, 4 no server could be reached or started,
5 wait-for: the program exited before the text showed.";

/// Run `ctl` with `args` (after `ctl`).
pub fn run(args: &[String]) -> i32 {
    let first = args.first().map(String::as_str);
    match first {
        None | Some("-h" | "--help" | "help") => {
            println!("{HELP}");
            return if first.is_none() { exit::USAGE } else { exit::OK };
        }
        Some("socket") => {
            println!(
                "{}",
                serde_json::json!({ "ok": true, "socket": protocol::socket_path(&protocol::dir()) })
            );
            return exit::OK;
        }
        Some("serve") => return super::serve_headless(),
        _ => {}
    }
    let req = match parse(args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("terminal-decline ctl: {e}\n(terminal-decline ctl --help for the commands)");
            return exit::USAGE;
        }
    };
    let screen_text = matches!(req, Request::Screen { json: false, .. });
    let wait = matches!(req, Request::WaitFor(_));
    let reply = match request(&protocol::dir(), &req) {
        Ok(r) => r,
        Err(e) => {
            println!("{}", serde_json::json!({ "ok": false, "error": e }));
            return exit::NO_SERVER;
        }
    };
    let ok = reply["ok"].as_bool() == Some(true);
    if screen_text && ok {
        for line in reply["lines"].as_array().into_iter().flatten() {
            println!("{}", line.as_str().unwrap_or(""));
        }
    } else {
        println!("{reply}");
    }
    if !ok {
        return exit::FAILED;
    }
    if wait && reply["matched"].as_bool() != Some(true) {
        return if reply["reason"] == "exited" { exit::EXITED } else { exit::TIMEOUT };
    }
    exit::OK
}

/// Send `req` to the server in `dir`, starting a headless one if none
/// answers.
pub fn request(dir: &Path, req: &Request) -> Result<Value, String> {
    let stream = connect(dir)?;
    let mut out = stream.try_clone().map_err(|e| e.to_string())?;
    let line = serde_json::to_string(req).map_err(|e| e.to_string())?;
    writeln!(out, "{line}").map_err(|e| format!("sending: {e}"))?;
    let mut reply = String::new();
    BufReader::new(stream)
        .read_line(&mut reply)
        .map_err(|e| format!("reading the reply: {e}"))?;
    serde_json::from_str(&reply).map_err(|e| format!("a bad reply ({e}): {reply:?}"))
}

fn connect(dir: &Path) -> Result<UnixStream, String> {
    let path = protocol::socket_path(dir);
    let missing = |e: &std::io::Error| matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused);
    match UnixStream::connect(&path) {
        Ok(s) => return Ok(s),
        Err(e) if missing(&e) => {}
        Err(e) => return Err(format!("{}: {e}", path.display())),
    }
    protocol::ensure_dir(dir).map_err(|e| e.to_string())?;
    let mut child = start_server(dir)?;
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        match UnixStream::connect(&path) {
            Ok(s) => {
                // Reaped by launchd once this process exits; a thread
                // reaps it should this one live on.
                std::thread::spawn(move || child.wait());
                return Ok(s);
            }
            Err(e) if missing(&e) && Instant::now() < until => {
                if let Ok(Some(_)) = child.try_wait() {
                    // It stopped: another one serves now, or it couldn't
                    // serve (its reason is the log's last line).
                    if let Ok(s) = UnixStream::connect(&path) {
                        return Ok(s);
                    }
                    let said = std::fs::read_to_string(dir.join("server.log")).unwrap_or_default();
                    let last = said
                        .lines()
                        .last()
                        .unwrap_or("")
                        .trim_start_matches("terminal-decline ctl: ")
                        .to_string();
                    return Err(format!("the headless server stopped: {last}"));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => {
                return Err(format!(
                    "no server at {} ({e}); see {}",
                    path.display(),
                    dir.join("server.log").display()
                ));
            }
        }
    }
}

/// Start `terminal-decline ctl serve` on its own: a session of its own,
/// stdio to the log.
fn start_server(dir: &Path) -> Result<std::process::Child, String> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().map_err(|e| format!("can't find this program: {e}"))?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("server.log"))
        .map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["ctl", "serve"])
        .env(protocol::DIR_ENV, dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(log);
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn().map_err(|e| format!("starting the headless server: {e}"))
}

fn size(s: &str) -> Result<(u16, u16), String> {
    let (c, r) = s.split_once(['x', 'X']).ok_or_else(|| format!("{s:?}: COLSxROWS, like 80x24"))?;
    let n = |v: &str| {
        v.parse::<u16>()
            .ok()
            .filter(|&n| n > 0)
            .ok_or_else(|| format!("{s:?}: COLSxROWS, like 80x24"))
    };
    Ok((n(c)?, n(r)?))
}

fn absolute(p: &str) -> String {
    let path = Path::new(p);
    if path.is_absolute() {
        return p.to_string();
    }
    std::env::current_dir()
        .map(|d| d.join(path).to_string_lossy().into_owned())
        .unwrap_or_else(|_| p.to_string())
}

/// Variables that say which terminal the client runs in: not passed on.
fn terminal_specific(key: &str) -> bool {
    matches!(
        key,
        "TERM"
            | "TERM_PROGRAM"
            | "TERM_PROGRAM_VERSION"
            | "COLORTERM"
            | "TERM_SESSION_ID"
            | "TMUX"
            | "TMUX_PANE"
            | "STY"
            | "WINDOWID"
            | "COLUMNS"
            | "LINES"
    ) || [
        "ITERM_",
        "KITTY_",
        "GHOSTTY_",
        "WEZTERM_",
        "ALACRITTY_",
        "VTE_",
        "TERMINAL_DECLINE_",
    ]
    .iter()
    .any(|p| key.starts_with(p))
}

fn mouse(spec: &str) -> Result<Mouse, String> {
    let bad = || format!("--mouse {spec:?}: KIND:LINE,COL[:BUTTON|:LINES], like click:3,10 or scroll:5,5:-3");
    let mut parts = spec.split(':');
    let kind = parts.next().ok_or_else(bad)?.to_string();
    let (line, col) = parts.next().and_then(|p| p.split_once(',')).ok_or_else(bad)?;
    let (line, col) = (line.parse().map_err(|_| bad())?, col.parse().map_err(|_| bad())?);
    let mut m = Mouse {
        kind,
        line,
        col,
        button: None,
        mods: None,
        lines: None,
    };
    if let Some(extra) = parts.next() {
        if m.kind == "scroll" {
            m.lines = Some(extra.parse().map_err(|_| bad())?);
        } else {
            m.button = Some(extra.to_string());
        }
    }
    if let Some(mods) = parts.next() {
        m.mods = Some(mods.to_string());
    }
    Ok(m)
}

/// The request for `args`.
pub fn parse(args: &[String]) -> Result<Request, String> {
    let cmd = args.first().ok_or("which command?")?.as_str();
    let mut rest = args[1..].iter();
    let value = |flag: &str, rest: &mut std::slice::Iter<String>| rest.next().cloned().ok_or_else(|| format!("{flag} needs a value"));
    let id = |rest: &mut std::slice::Iter<String>| -> Result<String, String> {
        match rest.next() {
            Some(s) if !s.starts_with("--") => Ok(s.clone()),
            _ => Err(format!("{cmd}: which session? (its ID first)")),
        }
    };
    let number = |flag: &str, v: String| v.parse::<u64>().map_err(|_| format!("{flag} {v:?}: a number"));
    let req = match cmd {
        "hello" => Request::Hello,
        "list" => Request::List,
        "spawn" => {
            let mut s = Spawn {
                cwd: std::env::current_dir().ok().map(|d| d.to_string_lossy().into_owned()),
                env: std::env::vars().filter(|(k, _)| !terminal_specific(k)).collect(),
                ..Spawn::default()
            };
            while let Some(a) = rest.next() {
                match a.as_str() {
                    "--" => {
                        s.argv = rest.by_ref().cloned().collect();
                        break;
                    }
                    "--name" => s.name = Some(value(a, &mut rest)?),
                    "--size" => (s.cols, s.rows) = size(&value(a, &mut rest)?)?,
                    "--cwd" => s.cwd = Some(absolute(&value(a, &mut rest)?)),
                    "--env" => {
                        let kv = value(a, &mut rest)?;
                        let (k, v) = kv.split_once('=').ok_or_else(|| format!("--env {kv:?}: KEY=VALUE"))?;
                        s.env.insert(k.into(), v.into());
                    }
                    "--term" => s.term = Some(value(a, &mut rest)?),
                    "--shell" => s.shell = true,
                    "--window" => s.window = true,
                    "--font" => s.font = Some(value(a, &mut rest)?),
                    "--font-size" => s.font_size = Some(value(a, &mut rest)?.parse().map_err(|_| "--font-size: a number")?),
                    "--theme" => s.theme = Some(value(a, &mut rest)?),
                    "--appearance" => s.appearance = Some(value(a, &mut rest)?),
                    "--scale" => s.scale = Some(value(a, &mut rest)?.parse().map_err(|_| "--scale: a number")?),
                    "--scrollback" => s.scrollback = Some(number(a, value(a, &mut rest)?)? as usize),
                    "--record" => s.record = Some(absolute(&value(a, &mut rest)?)),
                    other if !other.starts_with('-') => {
                        s.argv = std::iter::once(a).chain(rest.by_ref()).cloned().collect();
                        break;
                    }
                    other => return Err(format!("spawn: unknown option {other}")),
                }
            }
            Request::Spawn(s)
        }
        "send" => {
            let session = id(&mut rest)?;
            let mut actions = Vec::new();
            while let Some(a) = rest.next() {
                actions.push(match a.as_str() {
                    "--text" => Action::Text(value(a, &mut rest)?),
                    "--key" => {
                        let k = value(a, &mut rest)?;
                        super::keys::KeySpec::parse(&k)?;
                        Action::Key(k)
                    }
                    "--paste" => Action::Paste(value(a, &mut rest)?),
                    "--raw" => {
                        let r = value(a, &mut rest)?;
                        protocol::unescape(&r)?;
                        Action::Raw(r)
                    }
                    "--mouse" => Action::Mouse(mouse(&value(a, &mut rest)?)?),
                    "--sleep" => Action::Sleep(number(a, value(a, &mut rest)?)?),
                    other => {
                        return Err(format!(
                            "send: unknown action {other} (--text, --key, --paste, --raw, --mouse, --sleep)"
                        ));
                    }
                });
            }
            if actions.is_empty() {
                return Err("send: nothing to send".into());
            }
            Request::Send { session, actions }
        }
        "screen" => {
            let session = id(&mut rest)?;
            let (mut json, mut scrollback) = (false, 0);
            while let Some(a) = rest.next() {
                match a.as_str() {
                    "--json" => json = true,
                    "--scrollback" => scrollback = number(a, value(a, &mut rest)?)? as usize,
                    other => return Err(format!("screen: unknown option {other}")),
                }
            }
            Request::Screen { session, json, scrollback }
        }
        "wait-for" => {
            let mut w = WaitFor {
                session: id(&mut rest)?,
                ..WaitFor::default()
            };
            while let Some(a) = rest.next() {
                match a.as_str() {
                    "--text" => w.text = Some(value(a, &mut rest)?),
                    "--regex" => w.regex = Some(value(a, &mut rest)?),
                    "--idle" => w.idle_ms = Some(number(a, value(a, &mut rest)?)?),
                    "--exit" => w.exit = true,
                    "--command" => w.command = true,
                    "--timeout" => w.timeout_ms = Some(number(a, value(a, &mut rest)?)?),
                    other => return Err(format!("wait-for: unknown option {other}")),
                }
            }
            let given = [w.text.is_some(), w.regex.is_some(), w.idle_ms.is_some(), w.exit, w.command]
                .iter()
                .filter(|&&b| b)
                .count();
            if given != 1 {
                return Err("wait-for: one of --text, --regex, --idle, --exit or --command".into());
            }
            Request::WaitFor(w)
        }
        "resize" => {
            let session = id(&mut rest)?;
            let (cols, rows) = size(rest.next().ok_or("resize: COLSxROWS")?)?;
            Request::Resize { session, cols, rows }
        }
        "screenshot" => {
            let session = id(&mut rest)?;
            let path = absolute(rest.next().ok_or("screenshot: which file?")?);
            Request::Screenshot { session, path }
        }
        "stats" => Request::Stats { session: id(&mut rest)? },
        "record" => {
            let session = id(&mut rest)?;
            let (mut path, mut input, mut stop) = (None, false, false);
            for a in rest.by_ref() {
                match a.as_str() {
                    "--stop" => stop = true,
                    "--input" => input = true,
                    p if !p.starts_with("--") && path.is_none() => path = Some(absolute(p)),
                    other => return Err(format!("record: unknown option {other}")),
                }
            }
            if stop == path.is_some() {
                return Err("record: a FILE.cast, or --stop".into());
            }
            Request::Record { session, path, input }
        }
        "kill" => {
            let session = id(&mut rest)?;
            let mut signal = None;
            while let Some(a) = rest.next() {
                match a.as_str() {
                    "--signal" => signal = Some(value(a, &mut rest)?),
                    other => return Err(format!("kill: unknown option {other}")),
                }
            }
            Request::Kill { session, signal }
        }
        other => return Err(format!("unknown command {other:?}")),
    };
    if let Some(extra) = rest.next() {
        return Err(format!("{cmd}: unexpected {extra:?}"));
    }
    Ok(req)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(a: &[&str]) -> Result<Request, String> {
        parse(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn parses_commands() {
        let Ok(Request::Spawn(s)) = p(&[
            "spawn", "--name", "top", "--size", "100x30", "--env", "A=1", "--", "htop", "-d", "5",
        ]) else {
            panic!()
        };
        assert_eq!((s.name.as_deref(), s.cols, s.rows), (Some("top"), 100, 30));
        assert_eq!(s.argv, ["htop", "-d", "5"]);
        assert_eq!(s.env.get("A").map(String::as_str), Some("1"));
        assert!(!s.env.contains_key("TERM"));
        let Ok(Request::Spawn(s)) = p(&["spawn", "vim", "--clean"]) else {
            panic!()
        };
        assert_eq!(s.argv, ["vim", "--clean"]);
        assert_eq!(
            p(&["send", "1", "--text", "ls\n", "--key", "ctrl+c", "--mouse", "scroll:2,3:-4"]).unwrap(),
            Request::Send {
                session: "1".into(),
                actions: vec![
                    Action::Text("ls\n".into()),
                    Action::Key("ctrl+c".into()),
                    Action::Mouse(Mouse {
                        kind: "scroll".into(),
                        line: 2,
                        col: 3,
                        button: None,
                        mods: None,
                        lines: Some(-4)
                    })
                ]
            }
        );
        assert_eq!(
            p(&["resize", "x", "10x5"]).unwrap(),
            Request::Resize {
                session: "x".into(),
                cols: 10,
                rows: 5
            }
        );
        assert!(matches!(
            p(&["wait-for", "1", "--idle", "200"]),
            Ok(Request::WaitFor(WaitFor { idle_ms: Some(200), .. }))
        ));
        assert!(matches!(
            p(&["wait-for", "1", "--command", "--timeout", "500"]),
            Ok(Request::WaitFor(WaitFor {
                command: true,
                timeout_ms: Some(500),
                ..
            }))
        ));
        let Ok(Request::Screenshot { path, .. }) = p(&["screenshot", "1", "a.png"]) else {
            panic!()
        };
        assert!(path.starts_with('/') && path.ends_with("/a.png"));
        for bad in [
            &["frobnicate"][..],
            &["send", "1"],
            &["send", "--key", "x"],
            &["send", "1", "--key", "hyper+x"],
            &["wait-for", "1"],
            &["wait-for", "1", "--text", "a", "--exit"],
            &["wait-for", "1", "--command", "--exit"],
            &["resize", "1", "80"],
            &["spawn", "--size", "0x3"],
            &["record", "1"],
            &["stats", "1", "extra"],
        ] {
            assert!(p(bad).is_err(), "{bad:?}");
        }
    }
}
