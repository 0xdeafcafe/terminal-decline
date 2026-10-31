//! The agent API end to end (docs/design/agent-api.md): the real
//! `terminal-decline ctl` binary, a headless server it starts on demand
//! in a socket directory of the test's own (never the user's), and real
//! programs on real PTYs. Nothing here opens a window: the server has no
//! `NSApplication`, which [`not_an_app`] checks with Launch Services.
//!
//! HOME and the config are a temp dir's, so the user's config, shell and
//! dotfiles don't matter.
#![cfg(target_os = "macos")]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

const EXE: &str = env!("CARGO_BIN_EXE_terminal-decline");

/// A socket directory and HOME of the test's own, removed on drop. Short
/// paths: a socket's path must fit in 104 bytes.
struct Env {
    root: PathBuf,
}

impl Env {
    fn new(config: &str) -> Env {
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = PathBuf::from(format!("/tmp/tdctl-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("home/.config/terminal-decline")).unwrap();
        std::fs::write(root.join("home/.config/terminal-decline/config.toml"), config).unwrap();
        Env { root }
    }

    fn dir(&self) -> PathBuf {
        self.root.join("s")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(EXE);
        c.arg("ctl")
            .args(args)
            .env("TERMINAL_DECLINE_CTL_DIR", self.dir())
            .env("TERMINAL_DECLINE_CTL_IDLE_MS", "1500")
            .env("HOME", self.root.join("home"))
            .env_remove("XDG_CONFIG_HOME")
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
        c
    }

    /// Run `ctl args`: its exit status and output.
    fn run(&self, args: &[&str]) -> (i32, String) {
        let out = self.command(args).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        (out.status.code().unwrap_or(-1), text)
    }

    /// Run `ctl args`, which must succeed, and its JSON.
    fn ok(&self, args: &[&str]) -> Value {
        let (code, text) = self.run(args);
        assert_eq!(code, 0, "ctl {args:?}: {text}");
        let v: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("ctl {args:?}: {e}: {text}"));
        assert_eq!(v["ok"], true, "{v}");
        v
    }

    fn path(&self, name: &str) -> String {
        self.root.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        // The server exits on its own once idle.
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

const CONFIG: &str = "padding = 0\nfont_size = 13\n[colors]\nforeground = '#ffffff'\nbackground = '#102030'\n";

/// The server is no app: Launch Services doesn't know its pid, so it has
/// no Dock icon, no menu bar and no windows.
fn not_an_app(pid: u64) {
    let Ok(out) = Command::new("/usr/bin/lsappinfo").arg("find").arg(format!("pid={pid}")).output() else {
        return;
    };
    let found = String::from_utf8_lossy(&out.stdout);
    assert!(found.trim().is_empty(), "the headless server is an app: {found}");
}

#[test]
fn headless_sessions_end_to_end() {
    let env = Env::new(CONFIG);
    let hello = env.ok(&["hello"]);
    assert_eq!(hello["server"], "headless");
    let server = hello["pid"].as_u64().unwrap();

    // Two sessions at once, each its own terminal.
    let a = env.ok(&["spawn", "--name", "a", "--size", "40x6", "--", "/bin/sh"]);
    assert_eq!(a["session"], "a");
    env.ok(&["spawn", "--name", "b", "--size", "30x4", "--", "/bin/sh"]);
    let (code, text) = env.run(&["spawn", "--name", "a", "--", "/bin/sh"]);
    assert_eq!(code, 1, "a second session a: {text}");

    env.ok(&["send", "a", "--text", "PS1='$ '; printf '\\033[1;31mred\\033[0m %s\\n' $((6*7))\n"]);
    let w = env.ok(&["wait-for", "a", "--regex", "red 42", "--timeout", "5000"]);
    assert_eq!(w["matched"], true);
    env.ok(&["send", "b", "--text", "echo bee\n"]);
    env.ok(&["wait-for", "b", "--regex", "(?m)^bee$", "--timeout", "5000"]);
    let (code, _) = env.run(&["wait-for", "b", "--text", "never shown", "--timeout", "300"]);
    assert_eq!(code, 3, "a timeout");

    // The screen: text, and cells with resolved colours.
    let (code, text) = env.run(&["screen", "a"]);
    assert_eq!(code, 0);
    assert!(text.lines().any(|l| l == "red 42"), "{text}");
    let s = env.ok(&["screen", "a", "--json"]);
    assert_eq!((s["cols"].as_u64(), s["rows"].as_u64()), (Some(40), Some(6)));
    let row = s["lines"].as_array().unwrap().iter().position(|l| l == "red 42").unwrap();
    let r = &s["cells"][row][0];
    assert_eq!((r["t"].as_str(), r["bold"].as_bool()), (Some("r"), Some(true)));
    assert_eq!(s["cells"][row][4]["fg"], "#ffffff");
    assert_eq!(s["cells"][row][4]["bg"], "#102030");
    assert_eq!(s["alt_screen"], false);

    // Resize: the program is told.
    env.ok(&["resize", "a", "50x8"]);
    env.ok(&["send", "a", "--text", "stty size\n"]);
    env.ok(&["wait-for", "a", "--text", "8 50", "--timeout", "5000"]);

    // Recording, with input.
    let cast = env.path("a.cast");
    env.ok(&["record", "a", &cast, "--input"]);
    env.ok(&["send", "a", "--text", "echo recorded\n"]);
    env.ok(&["wait-for", "a", "--regex", "(?m)^recorded$", "--timeout", "5000"]);
    env.ok(&["record", "a", "--stop"]);
    let lines: Vec<Value> = std::fs::read_to_string(&cast)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["version"], 2);
    assert_eq!(lines[0]["width"], 50);
    assert!(lines[1..].iter().any(|e| e[1] == "i" && e[2] == "echo recorded\r"), "{lines:?}");
    let output: String = lines[1..].iter().filter(|e| e[1] == "o").map(|e| e[2].as_str().unwrap()).collect();
    assert!(output.contains("recorded\r\n"), "{output:?}");

    // Stats.
    let st = env.ok(&["stats", "a"]);
    assert!(
        st["bytes_in"].as_u64().unwrap() > 20 && st["bytes_out"].as_u64().unwrap() > 20,
        "{st}"
    );
    assert_eq!(st["running"], true);
    assert!(st["server"]["phys_footprint"].as_u64().unwrap() > 0);

    // Exit status, and the screen still readable after.
    env.ok(&["send", "a", "--text", "exit 7\n"]);
    let w = env.ok(&["wait-for", "a", "--exit", "--timeout", "5000"]);
    assert_eq!(w["exit"]["code"], 7);
    assert!(env.ok(&["screen", "a", "--json"])["lines"].to_string().contains("red 42"));
    let (code, _) = env.run(&["wait-for", "a", "--text", "never shown", "--timeout", "5000"]);
    assert_eq!(code, 5, "the program had exited");

    let list = env.ok(&["list"]);
    assert_eq!(list["sessions"].as_array().unwrap().len(), 2);
    let k = env.ok(&["kill", "b"]);
    assert!(k["exit"].is_object(), "{k}");
    let (code, _) = env.run(&["screen", "b"]);
    assert_eq!(code, 1, "b is gone");
    not_an_app(server);
}

/// `wait-for --command`: the next command a shell finishes, with its
/// status and duration from the OSC 133 marks.
#[test]
fn wait_for_the_next_command_to_finish() {
    let env = Env::new(CONFIG);
    // Its history stays out of the checkout the integration is read from.
    let history = format!("HISTFILE={}", env.path("history"));
    env.ok(&[
        "spawn",
        "--name",
        "sh",
        "--shell",
        "--env",
        "SHELL=/bin/zsh",
        "--env",
        &history,
        "--size",
        "80x10",
    ]);
    // The first prompt has been drawn once the output is quiet.
    env.ok(&["wait-for", "sh", "--idle", "700", "--timeout", "15000"]);
    // No history: zsh would save it in the checkout the integration is
    // read from.
    env.ok(&["send", "sh", "--text", "unset HISTFILE\n"]);
    env.ok(&["wait-for", "sh", "--idle", "700", "--timeout", "15000"]);
    env.ok(&["send", "sh", "--text", "sleep 1; (exit 3)\n"]);
    let w = env.ok(&["wait-for", "sh", "--command", "--timeout", "10000"]);
    assert_eq!((w["matched"].clone(), w["reason"].clone()), (true.into(), "command".into()), "{w}");
    assert_eq!(w["status"], 3, "{w}");
    let ms = w["duration_ms"].as_u64().unwrap();
    assert!((800..8000).contains(&ms), "{w}");
    assert_eq!(w["command"], "sleep 1; (exit 3)", "{w}");
    // The next one, a success.
    env.ok(&["send", "sh", "--text", "sleep 1; true\n"]);
    let w = env.ok(&["wait-for", "sh", "--command", "--timeout", "10000"]);
    assert_eq!(w["status"], 0, "{w}");
    // Nothing finishes: a timeout (3).
    let (code, _) = env.run(&["wait-for", "sh", "--command", "--timeout", "300"]);
    assert_eq!(code, 3);
    // A program run directly has no marks to wait on.
    env.ok(&["spawn", "--name", "plain", "--", "/bin/sh"]);
    let (code, text) = env.run(&["wait-for", "plain", "--command", "--timeout", "300"]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("--shell"), "{text}");
}

#[test]
fn keys_go_through_the_apps_encoding() {
    let env = Env::new(CONFIG);
    // The program asks for kitty level 1 (disambiguate), then shows the
    // bytes it reads.
    env.ok(&[
        "spawn",
        "--name",
        "k",
        "--",
        "/bin/sh",
        "-c",
        "printf '\\033[>1u'; stty raw -echo; printf READY; dd bs=1 count=13 2>/dev/null | od -An -c; sleep 10",
    ]);
    // Keys only go out once the program says it is in raw mode with kitty
    // level 1 on. A quiet screen proves nothing: a slow start looks the
    // same, and then Ctrl+C reaches the tty in cooked mode and kills the
    // shell with SIGINT.
    env.ok(&["wait-for", "k", "--text", "READY", "--timeout", "30000"]);
    env.ok(&["send", "k", "--key", "esc", "--key", "ctrl+c", "--key", "a"]);
    let w = env.ok(&["wait-for", "k", "--regex", "5\\s+u\\s+a", "--timeout", "30000"]);
    assert_eq!(w["matched"], true);
    let text = env.run(&["screen", "k"]).1;
    let flat: String = text.split_whitespace().collect();
    // ESC [ 2 7 u, ESC [ 9 9 ; 5 u, a
    assert!(flat.contains("033[27u033[99;5ua"), "{text}");
    env.ok(&["kill", "k"]);
}

#[test]
fn screenshots_draw_the_screen() {
    let env = Env::new(CONFIG);
    env.ok(&[
        "spawn",
        "--name",
        "p",
        "--size",
        "10x3",
        "--scale",
        "1",
        "--",
        "/bin/sh",
        "-c",
        "printf '\\033[?25l\\033[48;2;200;10;20m    \\033[0m\\r\\nMMMM'; sleep 10",
    ]);
    env.ok(&["wait-for", "p", "--text", "MMMM", "--timeout", "5000"]);
    let png_path = env.path("p.png");
    let shot = env.ok(&["screenshot", "p", &png_path]);
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&png_path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(
        (info.width as u64, info.height as u64),
        (shot["width"].as_u64().unwrap(), shot["height"].as_u64().unwrap())
    );
    let (w, h) = (info.width as usize, info.height as usize);
    let (cw, ch) = (w / 10, h / 3);
    let at = |x: usize, y: usize| -> [u8; 4] { buf[(y * w + x) * 4..][..4].try_into().unwrap() };
    // Padding 0: the grid fills the image.
    assert_eq!(at(cw / 2, ch / 2), [200, 10, 20, 255], "the red cell");
    assert_eq!(at(cw * 8, ch * 2 + ch / 2), [0x10, 0x20, 0x30, 255], "the background");
    let inked = (ch..2 * ch)
        .flat_map(|y| (0..4 * cw).map(move |x| (x, y)))
        .filter(|&(x, y)| at(x, y)[0] > 200)
        .count();
    assert!(inked > 20, "the Ms drew ({inked} white pixels)");
    let st = env.ok(&["stats", "p"]);
    assert!(st["screenshot"]["gpu_ms"].as_f64().is_some(), "{st}");
    env.ok(&["kill", "p"]);
}

/// Only this user can reach the socket: the directory is 0700, the
/// socket 0600, and a directory others can enter is refused.
#[test]
fn the_socket_is_the_users_alone() {
    let env = Env::new(CONFIG);
    env.ok(&["hello"]);
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&env.dir()), 0o700);
    assert_eq!(mode(&env.dir().join("ctl.sock")), 0o600);
    let socket = env.ok(&["socket"]);
    assert_eq!(socket["socket"], env.dir().join("ctl.sock").to_string_lossy().as_ref());

    let open = Env::new(CONFIG);
    std::fs::create_dir_all(open.dir()).unwrap();
    std::fs::set_permissions(open.dir(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let (code, text) = open.run(&["hello"]);
    assert_eq!(code, 4, "{text}");
    assert!(text.contains("0700"), "{text}");

    // agent_api = false: no server starts.
    let off = Env::new("agent_api = false\n");
    let (code, _) = off.run(&["hello"]);
    assert_eq!(code, 4);
}

#[test]
fn bad_arguments_exit_2() {
    let env = Env::new(CONFIG);
    for args in [&["frobnicate"][..], &["send", "1"], &["wait-for", "1"], &["resize", "1", "80"]] {
        assert_eq!(env.run(args).0, 2, "{args:?}");
    }
    let help = env.command(&["--help"]).output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("wait-for ID"));
}

/// Decode the PNG at `path`: width, height and RGBA bytes.
fn read_png(path: &str) -> (usize, usize, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    (info.width as usize, info.height as usize, buf)
}

/// Colours a program sets (OSC 10, 11, 4) are what the screen reports
/// and the screenshot draws, a colour query (OSC 4 ?) is answered, and
/// the resets bring the theme back. Ported from the window smoke test
/// `smoke_program_set_colours_are_drawn`: headless, no window.
#[test]
fn program_set_colours_are_drawn() {
    let env = Env::new(CONFIG);
    env.ok(&["spawn", "--name", "c", "--size", "20x4", "--scale", "1", "--", "/bin/sh"]);
    env.ok(&[
        "send",
        "c",
        "--text",
        "PS1='> '; printf '\\033]11;#203040\\007\\033]10;rgb:aa/bb/cc\\007\\033]4;1;#ff0000\\007'; echo set\n",
    ]);
    env.ok(&["wait-for", "c", "--regex", "(?m)^set$", "--timeout", "5000"]);
    let s = env.ok(&["screen", "c", "--json"]);
    assert_eq!(s["colors"]["background"], "#203040", "{s}");
    assert_eq!(s["colors"]["foreground"], "#aabbcc", "{s}");
    let shot = env.path("c.png");
    env.ok(&["screenshot", "c", &shot]);
    let (w, h, px) = read_png(&shot);
    let corner = &px[((h - 1) * w + (w - 1)) * 4..][..4];
    assert_eq!(corner, [0x20, 0x30, 0x40, 255], "the program's background");

    // Back to the theme, then ask for colour 1.
    let ask = "printf '\\033]110\\007\\033]111\\007\\033]104\\007'; stty raw -echo; printf '\\033]4;1;?\\007'; \
               reply=$(dd bs=1 count=24 2>/dev/null); stty sane; printf '%s\\n' \"$reply\" | tr -d '\\033\\007' | sed 's/^]/osc /'\n";
    env.ok(&["send", "c", "--text", ask]);
    env.ok(&["wait-for", "c", "--text", "osc 4;1;rgb:", "--timeout", "5000"]);
    let s = env.ok(&["screen", "c", "--json"]);
    assert_eq!(s["colors"]["background"], "#102030", "{s}");
    env.ok(&["kill", "c"]);
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A kitty graphics image is decoded and drawn in the screenshot, as a
/// window draws it.
#[test]
fn screenshots_draw_images() {
    let env = Env::new(&format!("{CONFIG}images = true\n"));
    // A 40 × 40 green PNG.
    let mut png_bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_bytes, 40, 40);
        enc.set_color(png::ColorType::Rgba);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[0, 200, 0, 255].repeat(40 * 40)).unwrap();
    }
    let file = env.path("img.b64");
    std::fs::write(&file, base64(&png_bytes)).unwrap();
    let script = format!("printf '\\033_Ga=T,f=100,q=2;%s\\033\\\\' \"$(cat {file})\"; echo; echo shown; sleep 10");
    env.ok(&[
        "spawn", "--name", "i", "--size", "20x6", "--scale", "1", "--", "/bin/sh", "-c", &script,
    ]);
    env.ok(&["wait-for", "i", "--text", "shown", "--timeout", "5000"]);
    let shot = env.path("i.png");
    // Decoding runs on a queue: a few tries.
    let mut green = 0;
    for _ in 0..50 {
        env.ok(&["screenshot", "i", &shot]);
        let (_, _, px) = read_png(&shot);
        // Green, whatever colour matching does to it.
        green = px
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[1] > 150 && p[1] as u32 > p[0] as u32 + 60 && p[1] as u32 > p[2] as u32 + 60)
            .count();
        if green >= 1000 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(green >= 1000, "the image drew ({green} green pixels)");
    env.ok(&["kill", "i"]);
}

/// Windows need the app to host them: a headless server refuses
/// `--window` and says why (the app's side is in the smoke tests).
#[test]
fn a_headless_server_refuses_windows() {
    let env = Env::new(CONFIG);
    let hello = env.ok(&["hello"]);
    assert_eq!(
        (hello["server"].as_str(), hello["windows"].as_bool()),
        (Some("headless"), Some(false))
    );
    let (code, text) = env.run(&["spawn", "--window", "--name", "w", "--", "/bin/sh"]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("--window needs the app"), "{text}");
    assert_eq!(env.ok(&["list"])["sessions"].as_array().unwrap().len(), 0);
}
