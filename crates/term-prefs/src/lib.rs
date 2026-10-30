//! The preferences: a terminal program Terminal Decline runs in a tab of
//! its own (`terminal-decline prefs`, Cmd+,). No platform code: it edits
//! `config.toml` and draws a TUI, and gets what it needs from the app
//! through [`Host`]. See docs/design/preferences.md.
//!
//! - [`schema`]: every setting, its section, editor and whether running
//!   windows take it on.
//! - [`file`]: safe edits to a config file with `toml_edit`.
//! - [`layers`]: the values shown, and where each comes from.
//! - [`App`]: the TUI's state and frames; [`run`] drives it in a real
//!   terminal.

pub mod app;
pub mod file;
pub mod host;
pub mod layers;
pub mod schema;
pub mod ui;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

pub use app::{App, Effect};
pub use host::{Action, Checked, Fake, Font, Host, Pin, ThemeInfo};

use ui::frame::Renderer;
use ui::input::Decoder;
use ui::tty::{self, Tty, Wake};

/// What `prefs` was asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    /// The files the window's config is read from, lowest first. Empty:
    /// the user's own file.
    pub layers: Vec<PathBuf>,
    /// The layer being edited (an index into `layers`).
    pub edit: usize,
    /// The app is there to record chords (OSC 7771).
    pub in_app: bool,
}

pub const USAGE: &str = "usage: terminal-decline prefs [--layer FILE]... [--edit N] [--in-app]

Edits the settings in config.toml (or the file --edit names among the
--layer files, lowest first) in this terminal.";

impl Args {
    /// Parses the arguments after `prefs`. `--layer` may repeat or take
    /// several files.
    pub fn parse(args: &[String]) -> Result<Args, String> {
        let mut out = Args {
            layers: Vec::new(),
            edit: usize::MAX,
            in_app: false,
        };
        let mut it = args.iter().peekable();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--layer" => {
                    let mut any = false;
                    while let Some(p) = it.next_if(|p| !p.starts_with("--")) {
                        out.layers.push(PathBuf::from(p));
                        any = true;
                    }
                    if !any {
                        return Err("--layer needs a file".into());
                    }
                }
                "--edit" => {
                    let n = it.next().ok_or("--edit needs a number")?;
                    out.edit = n.parse().map_err(|_| format!("--edit {n}: not a layer number"))?;
                }
                "--in-app" => out.in_app = true,
                "-h" | "--help" => return Err(USAGE.into()),
                other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
            }
        }
        if out.edit == usize::MAX {
            out.edit = out.layers.len().saturating_sub(1);
        } else if out.edit >= out.layers.len().max(1) {
            return Err(format!("--edit {}: there are only {} layers", out.edit, out.layers.len()));
        }
        Ok(out)
    }
}

/// Runs the preferences in this terminal until `q`; returns the exit code.
/// `args` are those after `prefs`.
pub fn run(args: &[String], host: &dyn Host) -> i32 {
    let mut args = match Args::parse(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    if args.layers.is_empty() {
        args.layers.push(host.user_config());
        args.edit = 0;
    }
    let tty = match Tty::open() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("terminal-decline prefs: {e}");
            return 1;
        }
    };
    install_panic_hook();
    let mut app = App::new(host, args.layers, args.edit, args.in_app);
    let (w, h) = tty.size();
    app.resize(w, h);
    let mut renderer = Renderer::default();
    let mut decoder = Decoder::default();
    loop {
        let mut quit = false;
        for effect in app.take_effects() {
            match effect {
                Effect::Quit => quit = true,
                Effect::Repaint => renderer.invalidate(),
                Effect::Send(s) => {
                    let _ = tty.write(&s);
                }
                Effect::Watch(files) => host.watch(&files, Arc::new(tty::file_changed)),
                Effect::EditFile(path) => {
                    tty.suspend();
                    let editor = std::env::var("VISUAL")
                        .ok()
                        .or_else(|| std::env::var("EDITOR").ok())
                        .filter(|e| !e.trim().is_empty())
                        .unwrap_or_else(|| "vi".into());
                    let status = std::process::Command::new("/bin/sh")
                        .arg("-c")
                        .arg(format!("{editor} \"$1\""))
                        .arg("sh")
                        .arg(&path)
                        .status();
                    let _ = tty.resume();
                    renderer.invalidate();
                    app.reload();
                    if let Err(e) = status {
                        eprintln!("{editor}: {e}");
                    }
                }
            }
        }
        if quit {
            app.flush(Instant::now());
            return 0;
        }
        let frame = app.draw();
        if tty.write(&renderer.render(&frame)).is_err() {
            return 1;
        }
        let timeout = app.deadline().map(|d| d.saturating_duration_since(Instant::now()));
        match tty.wait(timeout) {
            Ok(Wake::Input(bytes)) => {
                for e in decoder.feed(&bytes) {
                    app.handle(e, Instant::now());
                }
            }
            Ok(Wake::Resize) => {
                let (w, h) = tty.size();
                app.resize(w, h);
            }
            Ok(Wake::Changed) => app.reload(),
            Ok(Wake::Timeout) => app.tick(Instant::now()),
            Ok(Wake::Hangup) | Ok(Wake::Eof) | Err(_) => {
                // The tab is closing: write what is held back, then go.
                app.flush(Instant::now());
                return 0;
            }
        }
    }
}

/// On a panic: put the terminal back, show the message, and wait for a
/// key so the tab doesn't just vanish.
fn install_panic_hook() {
    let old = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tty::restore();
        old(info);
        eprintln!("\r\nterminal-decline prefs stopped. Press a key to close.");
        let mut b = [0u8; 1];
        // SAFETY: a blocking one-byte read from stdin.
        unsafe {
            libc::read(libc::STDIN_FILENO, b.as_mut_ptr().cast(), 1);
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Result<Args, String> {
        Args::parse(&s.split_whitespace().map(String::from).collect::<Vec<_>>())
    }

    #[test]
    fn arguments() {
        assert_eq!(
            args("").unwrap(),
            Args {
                layers: vec![],
                edit: 0,
                in_app: false
            }
        );
        let a = args("--layer a.toml b.toml --edit 0 --in-app").unwrap();
        assert_eq!((a.layers.len(), a.edit, a.in_app), (2, 0, true));
        let a = args("--layer a.toml --layer b.toml").unwrap();
        assert_eq!((a.layers.len(), a.edit), (2, 1));
        assert!(args("--layer a.toml --edit 3").is_err());
        assert!(args("--edit").is_err());
        assert!(args("--bogus").is_err());
    }
}
