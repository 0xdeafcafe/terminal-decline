//! Sessions shown in a window (`ctl spawn --window`, docs/design/agent-api.md
//! §6): the server, in the app, asks the app to open a window around the
//! session's program, and keeps what the window's pane hands back to drive
//! it. The window is the app's own, drawn as any other, in the background
//! ([`crate::background`]): it takes no focus.
//!
//! Only the app can host windows. The server thread asks through the
//! [`Opener`] the app installs ([`set_opener`]); a headless server has
//! none and refuses.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};

use term_core::Term;
use term_pty::{Child, Command};

use crate::config::Config;
use crate::session::{Events, Hooks, OnOutput, Session};

/// The env var that makes the app a host for `--window` sessions, started
/// by `ctl` when no app serves: the app runs in the background, opens no
/// window of its own, serves the socket, and leaves once idle.
pub const HOSTED_ENV: &str = "TERMINAL_DECLINE_CTL_HOSTED";

/// Whether this process is an app started by `ctl` to host windows.
pub fn hosted() -> bool {
    static HOSTED: OnceLock<bool> = OnceLock::new();
    *HOSTED.get_or_init(|| std::env::var_os(HOSTED_ENV).is_some_and(|v| !v.is_empty() && v != "0"))
}

/// What the server does to a session's window, from any thread; the app
/// answers on its main thread.
pub trait WindowControl: Send + Sync {
    /// Resize the window to show `cols` × `rows`.
    fn resize(&self, cols: u16, rows: u16) -> Result<(), String>;
    /// Close the window, which hangs up on the program.
    fn close(&self);
}

/// What the server watches a session with (from the reader thread).
pub struct Observer {
    /// The program's process, set once the session has started, for its
    /// exit status.
    pub child: Arc<OnceLock<Child>>,
    /// The program's output has ended.
    pub on_exit: Box<dyn FnOnce() + Send>,
    /// Commands the shell started and finished, and the rest of what the
    /// program asked for.
    pub on_events: Box<dyn Fn(&Events) + Send>,
    /// Each batch of output, after it was parsed, and the time that took.
    pub on_output: OnOutput,
}

/// What the server asks the app for.
pub struct WindowRequest {
    /// The app's config with the request's font, theme and size on top.
    pub config: Config,
    /// The program to run (the integration is added by the window).
    pub command: Command,
    pub cols: u16,
    pub rows: u16,
    pub observer: Observer,
}

/// What a window's pane hands back.
pub struct Ready {
    pub term: Arc<Mutex<Term>>,
    pub input: Option<Sender<Vec<u8>>>,
    pub child: Option<Child>,
    pub pid: Option<i32>,
}

/// The window the app opened.
pub struct Opened {
    pub ready: Ready,
    pub control: Arc<dyn WindowControl>,
}

/// Opens a window for a session: called on a server thread, it waits for
/// the main thread.
pub type Opener = Box<dyn Fn(WindowRequest) -> Result<Opened, String> + Send + Sync>;

static OPENER: OnceLock<Opener> = OnceLock::new();

/// The app can open windows: `opener` does it.
pub fn set_opener(opener: Opener) {
    let _ = OPENER.set(opener);
}

/// The app's way of opening a window, if this process is the app.
pub fn opener() -> Option<&'static Opener> {
    OPENER.get()
}

/// A window's pane about to start its session on the app's behalf.
pub struct Adopt {
    pub observer: Observer,
    /// Called with the session once it has started.
    pub ready: Box<dyn FnOnce(Ready) + Send>,
}

impl Adopt {
    /// Put the observer on `hooks`, after the window's own: the window
    /// still hears everything. Returns what to call with the session
    /// once it has started.
    pub fn attach(self, hooks: &mut Hooks) -> Box<dyn FnOnce(&Session)> {
        let Adopt {
            observer:
                Observer {
                    child,
                    on_exit,
                    on_events,
                    on_output,
                },
            ready,
        } = self;
        let window_exit = std::mem::replace(&mut hooks.on_exit, Box::new(|_| {}));
        hooks.on_exit = Box::new(move |outcome| {
            on_exit();
            window_exit(outcome);
        });
        let window_events = hooks.on_events.take();
        hooks.on_events = Some(Box::new(move |events| {
            on_events(&events);
            if let Some(f) = &window_events {
                f(events);
            }
        }));
        let window_output = hooks.on_output.take();
        hooks.on_output = Some(match window_output {
            None => on_output,
            Some(mut window) => {
                let mut ours = on_output;
                Box::new(move |bytes, took| {
                    ours(bytes, took);
                    window(bytes, took);
                })
            }
        });
        Box::new(move |session| {
            if let Some(c) = session.child() {
                let _ = child.set(c);
            }
            ready(Ready {
                term: session.term().clone(),
                input: session.input(),
                child: session.child(),
                pid: session.pid(),
            });
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Outcome;
    use std::time::Duration;

    /// The server's observer hears everything the window's hooks do, and
    /// first; the window still hears it all.
    #[test]
    fn the_observer_is_added_to_the_windows_hooks() {
        let log: Arc<Mutex<Vec<String>>> = Arc::default();
        let note = |what: &'static str| {
            let log = log.clone();
            move || log.lock().unwrap().push(what.to_string())
        };
        let (window_exit, ours_exit) = (note("window exit"), note("server exit"));
        let mut hooks = Hooks::new(Arc::new(|| {}), Box::new(move |_| window_exit()));
        let (window_events, ours_events) = (note("window events"), note("server events"));
        hooks.on_events = Some(Box::new(move |_| window_events()));
        let (window_output, ours_output) = (note("window output"), note("server output"));
        hooks.on_output = Some(Box::new(move |_, _| window_output()));
        let adopt = Adopt {
            observer: Observer {
                child: Arc::default(),
                on_exit: Box::new(ours_exit),
                on_events: Box::new(move |_| ours_events()),
                on_output: Box::new(move |_, _| ours_output()),
            },
            ready: Box::new(|_| {}),
        };
        let _ready = adopt.attach(&mut hooks);

        (hooks.on_output.as_mut().unwrap())(b"x", Duration::ZERO);
        (hooks.on_events.as_ref().unwrap())(Events::default());
        (hooks.on_exit)(Outcome::Closed);
        assert_eq!(
            *log.lock().unwrap(),
            [
                "server output",
                "window output",
                "server events",
                "window events",
                "server exit",
                "window exit"
            ]
        );
    }

    /// A window with no output hook of its own gets just the server's.
    #[test]
    fn the_observer_alone_when_the_window_has_no_output_hook() {
        let seen = Arc::new(Mutex::new(0));
        let s = seen.clone();
        let mut hooks = Hooks::new(Arc::new(|| {}), Box::new(|_| {}));
        let adopt = Adopt {
            observer: Observer {
                child: Arc::default(),
                on_exit: Box::new(|| {}),
                on_events: Box::new(|_| {}),
                on_output: Box::new(move |b, _| *s.lock().unwrap() += b.len()),
            },
            ready: Box::new(|_| {}),
        };
        let _ready = adopt.attach(&mut hooks);
        (hooks.on_output.as_mut().unwrap())(b"abc", Duration::ZERO);
        assert_eq!(*seen.lock().unwrap(), 3);
    }
}
