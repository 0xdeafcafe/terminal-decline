//! A headless session: the app's [`Session`] with no view, its terminal
//! made as a window's pane makes it, plus what the agent API counts and
//! waits on (docs/design/agent-api.md §3).

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use term_core::Term;
use term_pty::{Child, Command, Size};
use term_render::Theme;

use super::protocol::{Action, Mouse, Spawn, WaitFor};
use super::window::{Observer, Opener, Ready, WindowControl, WindowRequest};
use crate::config::Config;
use crate::session::{Hooks, Session};
use crate::theme::Appearance;

/// How a session looks: its config (fonts, theme, padding), the theme
/// resolved for its appearance, and the scale its pixels are at.
#[derive(Clone)]
pub struct Look {
    pub config: Config,
    pub theme: Theme,
    pub scale: f64,
    pub metrics: term_font::Metrics,
}

impl Look {
    /// `config` with `spawn`'s font, theme and scale on top.
    pub fn new(mut config: Config, spawn: &Spawn, themes_dir: &std::path::Path) -> Result<Look, String> {
        if let Some(f) = &spawn.font {
            config.font_family = f.clone();
        }
        if let Some(s) = spawn.font_size {
            if !(s.is_finite() && s > 0.0 && s < 500.0) {
                return Err(format!("font size {s} is out of range"));
            }
            config.font_size = s;
        }
        if let Some(t) = &spawn.theme {
            crate::theme::load(themes_dir, t).map_err(|e| format!("theme {t:?}: {e}"))?;
            config = config.previewing(t, themes_dir);
        }
        let appearance = match spawn.appearance.as_deref() {
            Some("light") => Appearance::Light,
            Some("dark") => Appearance::Dark,
            Some(other) => return Err(format!("appearance {other:?}: light or dark")),
            None => config.appearance.resolve(crate::prefs::system_appearance()),
        };
        let scale = spawn.scale.unwrap_or(2.0);
        if !(scale.is_finite() && (0.5..=4.0).contains(&scale)) {
            return Err(format!("scale {scale}: 0.5 to 4"));
        }
        let theme = config.theme(appearance);
        let metrics = metrics(&config, scale);
        Ok(Look {
            config,
            theme,
            scale,
            metrics,
        })
    }

    pub fn size(&self, cols: u16, rows: u16) -> Size {
        Size {
            cols,
            lines: rows,
            cell_width: self.metrics.cell_width as u16,
            cell_height: self.metrics.cell_height as u16,
        }
    }
}

/// The cell metrics of `config`'s fonts at `scale`, loaded once per
/// font settings.
fn metrics(config: &Config, scale: f64) -> term_font::Metrics {
    static CACHE: OnceLock<Mutex<HashMap<String, term_font::Metrics>>> = OnceLock::new();
    let fc = config.font_config(config.font_size);
    let key = format!("{fc:?}@{scale}");
    let cache = CACHE.get_or_init(Default::default);
    if let Some(m) = lock(cache).get(&key) {
        return *m;
    }
    let m = term_font::Fonts::with_config(&fc, scale).metrics;
    lock(cache).insert(key, m);
    m
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How the program ended.
#[derive(Clone, Copy, Debug)]
pub struct Exit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub after: Duration,
}

/// A command a shell finished, from its OSC 133 marks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Done {
    /// Its exit status, if the shell gave one.
    pub status: Option<u32>,
    /// From the mark that started it to the mark that finished it, as the
    /// terminal saw them (to a batch of output's precision).
    pub duration: Duration,
    /// Its text, when the shell integration could read it off the screen.
    pub command: Option<String>,
}

/// The most finished commands kept for `wait-for --command` to find.
const DONE_KEPT: usize = 16;

/// The commands a shell session ran: the one running, and the ones
/// finished, numbered from 1.
#[derive(Debug, Default)]
struct Commands {
    running: Option<(Instant, Option<String>)>,
    finished: u64,
    done: std::collections::VecDeque<(u64, Done)>,
}

impl Commands {
    /// Take in the marks of one batch of output, seen at `now`.
    fn apply(&mut self, marks: &[term_core::Command], now: Instant) {
        use term_core::Command;
        for mark in marks {
            match mark {
                Command::Prompt | Command::CommandLine(_) => {}
                Command::Started(text) => self.running = Some((now, text.clone())),
                Command::Finished(status) => {
                    let (since, command) = self.running.take().unwrap_or((now, None));
                    self.finished += 1;
                    if self.done.len() == DONE_KEPT {
                        self.done.pop_front();
                    }
                    self.done.push_back((
                        self.finished,
                        Done {
                            status: *status,
                            duration: now.saturating_duration_since(since),
                            command,
                        },
                    ));
                }
            }
        }
    }

    /// The first command to finish after `seen` commands had.
    fn after(&self, seen: u64) -> Option<&Done> {
        self.done.iter().find(|(n, _)| *n > seen).map(|(_, d)| d)
    }
}

/// The last screenshot's times.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShotTimes {
    pub build: Duration,
    pub gpu: Duration,
    pub total: Duration,
    pub width: u32,
    pub height: u32,
}

/// What the reader thread updates and waiters wait on.
#[derive(Default)]
struct Counts {
    bytes_in: u64,
    bytes_out: u64,
    batches: u64,
    parse: Duration,
    parse_max: Duration,
    last_output: Option<Instant>,
    first_output: Option<Duration>,
    exit: Option<Exit>,
    recorder: Option<Recorder>,
    shot: Option<ShotTimes>,
    commands: Commands,
}

struct Shared {
    counts: Mutex<Counts>,
    changed: Condvar,
}

/// An asciicast v2 file being written.
struct Recorder {
    out: BufWriter<File>,
    path: String,
    start: Instant,
    input: bool,
    /// The end of a UTF-8 sequence split across batches.
    carry: Vec<u8>,
}

impl Recorder {
    fn start(path: &str, cols: u16, rows: u16, term: &str, input: bool) -> Result<Recorder, String> {
        let file = File::create(path).map_err(|e| format!("{path}: {e}"))?;
        let mut r = Recorder {
            out: BufWriter::new(file),
            path: path.into(),
            start: Instant::now(),
            input,
            carry: Vec::new(),
        };
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let header = json!({ "version": 2, "width": cols, "height": rows, "timestamp": ts, "env": { "TERM": term } });
        r.line(&header);
        Ok(r)
    }

    fn line(&mut self, v: &Value) {
        let _ = writeln!(self.out, "{v}");
        let _ = self.out.flush();
    }

    fn event(&mut self, kind: &str, data: &str) {
        let t = self.start.elapsed().as_secs_f64();
        self.line(&json!([(t * 1e6).round() / 1e6, kind, data]));
    }

    /// Output: whole UTF-8 sequences only, the rest kept for the next.
    fn output(&mut self, bytes: &[u8]) {
        let mut all = std::mem::take(&mut self.carry);
        all.extend_from_slice(bytes);
        let cut = utf8_prefix(&all);
        self.carry = all.split_off(cut);
        if !all.is_empty() {
            self.event("o", &String::from_utf8_lossy(&all));
        }
    }
}

/// How much of `b` ends on a whole UTF-8 sequence: an incomplete one at
/// the end (up to 3 bytes) is left out.
fn utf8_prefix(b: &[u8]) -> usize {
    for back in 1..=3.min(b.len()) {
        let i = b.len() - back;
        let lead = b[i];
        if lead & 0xC0 == 0x80 {
            continue;
        }
        let need = match lead {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 1,
        };
        return if need > back { i } else { b.len() };
    }
    b.len()
}

/// How a session is served: the app's own [`Session`] with no view, or
/// a window's (`--window`), reached through what its pane gave.
enum Link {
    Own(Session),
    Window {
        input: Option<Sender<Vec<u8>>>,
        control: Arc<dyn WindowControl>,
    },
}

pub struct Headless {
    pub name: String,
    pub argv: Vec<String>,
    pub look: Look,
    term_name: String,
    link: Mutex<Option<Link>>,
    term: Arc<Mutex<Term>>,
    child: Option<Child>,
    pid: Option<i32>,
    shared: Arc<Shared>,
    started: Instant,
    spawn_time: Duration,
    /// The shell has the integration, so its commands are marked.
    integrated: bool,
    /// Shown in a window.
    window: bool,
}

/// What `spawn` asks of the program, worked out from the request.
struct Plan {
    command: Command,
    argv: Vec<String>,
    term_name: String,
    /// The shell gets the integration (as a window's does), so its
    /// commands are marked with where they start and end.
    integration: Option<std::path::PathBuf>,
}

impl Plan {
    /// `env` is the request's environment.
    fn new(spawn: &Spawn, look: &Look) -> Result<Plan, String> {
        if spawn.cols == 0 || spawn.rows == 0 || spawn.cols > 1000 || spawn.rows > 1000 {
            return Err(format!("size {}x{}: 1 to 1000 each", spawn.cols, spawn.rows));
        }
        let config = &look.config;
        let mut env: Vec<(String, String)> = spawn.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let term_name = spawn.term.clone().unwrap_or_else(|| "xterm-256color".into());
        env.retain(|(k, _)| k != "TERM");
        env.push(("TERM".into(), term_name.clone()));
        let mut argv = spawn.argv.clone();
        let direct = !spawn.shell && !argv.is_empty();
        if direct {
            let path = spawn
                .env
                .get("PATH")
                .cloned()
                .or_else(|| std::env::var("PATH").ok())
                .unwrap_or_default();
            argv[0] = which(&argv[0], &path, spawn.cwd.as_deref()).ok_or_else(|| format!("{:?}: not found on PATH", argv[0]))?;
        }
        let integration = if !direct && config.shell_integration {
            crate::Paths::current().shell_integration_dir()
        } else {
            None
        };
        let command = Command {
            argv: argv.clone(),
            cwd: spawn.cwd.clone(),
            env,
            shell: None,
            direct,
            integration: None,
        };
        Ok(Plan {
            command,
            argv,
            term_name,
            integration,
        })
    }
}

/// What the reader thread of a session updates, however it is served.
struct Watch {
    shared: Arc<Shared>,
    child: Arc<OnceLock<Child>>,
    started: Instant,
}

impl Watch {
    fn new(spawn: &Spawn, plan: &Plan) -> Result<Watch, String> {
        let shared = Arc::new(Shared {
            counts: Mutex::new(Counts::default()),
            changed: Condvar::new(),
        });
        if let Some(path) = &spawn.record {
            lock(&shared.counts).recorder = Some(Recorder::start(path, spawn.cols, spawn.rows, &plan.term_name, false)?);
        }
        Ok(Watch {
            shared,
            child: Arc::default(),
            started: Instant::now(),
        })
    }

    /// The callbacks the session's hooks are given.
    fn observer(&self) -> Observer {
        let (s1, s2, s3, slot, started) = (
            self.shared.clone(),
            self.shared.clone(),
            self.shared.clone(),
            self.child.clone(),
            self.started,
        );
        Observer {
            child: self.child.clone(),
            on_exit: Box::new(move || {
                // The output has ended; the status follows soon after.
                let mut status = None;
                let until = Instant::now() + Duration::from_secs(2);
                while status.is_none() && Instant::now() < until {
                    // The slot is set as the session starts: a program
                    // that exits at once can beat it.
                    status = slot.get().and_then(|child| child.try_wait());
                    if status.is_none() {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                }
                use std::os::unix::process::ExitStatusExt;
                let mut c = lock(&s1.counts);
                c.exit = Some(Exit {
                    code: status.and_then(|s| s.code()),
                    signal: status.and_then(|s| s.signal()),
                    after: started.elapsed(),
                });
                drop(c);
                s1.changed.notify_all();
            }),
            on_events: Box::new(move |events| {
                if !events.commands.is_empty() {
                    lock(&s3.counts).commands.apply(&events.commands, Instant::now());
                    s3.changed.notify_all();
                }
            }),
            on_output: Box::new(move |bytes, took| {
                let mut c = lock(&s2.counts);
                c.bytes_in += bytes.len() as u64;
                c.batches += 1;
                c.parse += took;
                c.parse_max = c.parse_max.max(took);
                let now = Instant::now();
                c.last_output = Some(now);
                c.first_output.get_or_insert(now - started);
                if let Some(r) = &mut c.recorder {
                    r.output(bytes);
                }
                drop(c);
                s2.changed.notify_all();
            }),
        }
    }
}

impl Headless {
    /// Start `spawn`'s program, with no window.
    pub fn spawn(name: String, spawn: &Spawn, look: Look) -> Result<Headless, String> {
        let mut plan = Plan::new(spawn, &look)?;
        let config = &look.config;
        // A program run directly gets exactly what it prints; a shell
        // (`--shell`) gets the integration, as a window's does.
        let integrated = plan.integration.is_some();
        plan.command.integration = plan.integration.take();
        let scrollback = spawn.scrollback.unwrap_or(config.scrollback);
        let mut t = Term::new(
            spawn.cols as usize,
            spawn.rows as usize,
            scrollback,
            term_core::ImagePool::process(),
        );
        t.state.set_ambiguous_width(config.ambiguous_width);
        crate::config::set_term_colors(&mut t.state, &look.theme);
        t.state
            .set_cell_pixels((look.metrics.cell_width as u16, look.metrics.cell_height as u16));
        t.state.set_image_scale(look.scale as f32);

        let watch = Watch::new(spawn, &plan)?;
        let Observer {
            on_exit,
            on_events,
            on_output,
            ..
        } = watch.observer();
        let mut hooks = Hooks::new(Arc::new(|| {}), Box::new(move |_| on_exit()));
        hooks.quiet_exit = true;
        hooks.on_events = Some(Box::new(move |events| on_events(&events)));
        hooks.on_output = Some(on_output);
        let size = look.size(spawn.cols, spawn.rows);
        let session = Session::spawn(&plan.command, size, t, hooks).map_err(|e| format!("couldn't start the session: {e}"))?;
        let spawn_time = watch.started.elapsed();
        let child = session.child();
        if let Some(c) = &child {
            let _ = watch.child.set(c.clone());
        }
        let pid = session.pid();
        if pid.is_none() {
            let text = super::screen::lines(&mut session.lock_term().state, 0).join("\n");
            return Err(text.trim().to_string());
        }
        Ok(Headless {
            name,
            argv: plan.argv,
            look,
            term_name: plan.term_name,
            term: session.term().clone(),
            link: Mutex::new(Some(Link::Own(session))),
            child,
            pid,
            shared: watch.shared,
            started: watch.started,
            spawn_time,
            integrated,
            window: false,
        })
    }

    /// Start `spawn`'s program in a window of the app, through `open`.
    /// `look` is what the window is made with and what screenshots draw.
    pub fn spawn_window(name: String, spawn: &Spawn, look: Look, open: &Opener) -> Result<Headless, String> {
        let plan = Plan::new(spawn, &look)?;
        let integrated = plan.integration.is_some();
        let watch = Watch::new(spawn, &plan)?;
        let opened = open(WindowRequest {
            config: look.config.clone(),
            command: plan.command.clone(),
            cols: spawn.cols,
            rows: spawn.rows,
            observer: watch.observer(),
        })?;
        let spawn_time = watch.started.elapsed();
        let Ready { term, input, child, pid } = opened.ready;
        if let Some(c) = &child {
            let _ = watch.child.set(c.clone());
        }
        Ok(Headless {
            name,
            argv: plan.argv,
            look,
            term_name: plan.term_name,
            term,
            link: Mutex::new(Some(Link::Window {
                input,
                control: opened.control,
            })),
            child,
            pid,
            shared: watch.shared,
            started: watch.started,
            spawn_time,
            integrated,
            window: true,
        })
    }

    pub fn term(&self) -> MutexGuard<'_, Term> {
        crate::session::lock(&self.term)
    }

    pub fn running(&self) -> bool {
        lock(&self.shared.counts).exit.is_none()
    }

    fn write(&self, bytes: Vec<u8>) -> Result<(), String> {
        if bytes.is_empty() {
            return Ok(());
        }
        let n = bytes.len() as u64;
        let mut c = lock(&self.shared.counts);
        if let Some(r) = c.recorder.as_mut().filter(|r| r.input) {
            r.event("i", &String::from_utf8_lossy(&bytes));
        }
        c.bytes_out += n;
        drop(c);
        match &*lock(&self.link) {
            Some(Link::Own(s)) => {
                s.send(bytes);
                Ok(())
            }
            Some(Link::Window { input, .. }) => {
                if let Some(tx) = input {
                    let _ = tx.send(bytes);
                }
                Ok(())
            }
            None => Err("the session was killed".into()),
        }
    }

    /// Do `actions` in order.
    pub fn send(&self, actions: &[Action]) -> Result<Value, String> {
        let mut sent = 0;
        for a in actions {
            let modes = super::keys::modes(&self.term().state);
            let mut out = Vec::new();
            match a {
                Action::Text(t) => super::keys::text_bytes(t, modes, &mut out),
                Action::Key(k) => {
                    let key = super::keys::KeySpec::parse(k)?;
                    super::keys::key_bytes(&key, modes, &mut out).map_err(|e| format!("{k}: {e}"))?;
                }
                Action::Paste(p) => {
                    let bracketed = self.term().state.modes().bracketed_paste;
                    out = crate::keys::paste_bytes(p, bracketed);
                }
                Action::Raw(r) => out = super::protocol::unescape(r)?,
                Action::Mouse(m) => out = self.mouse(m)?,
                Action::Sleep(ms) => std::thread::sleep(Duration::from_millis((*ms).min(60_000))),
            }
            sent += out.len();
            self.write(out)?;
        }
        Ok(json!({ "ok": true, "bytes": sent }))
    }

    fn mouse(&self, m: &Mouse) -> Result<Vec<u8>, String> {
        use crate::mouse as ms;
        let t = self.term();
        let modes = *t.state.modes();
        if modes.mouse == term_core::MouseMode::Off {
            return Err("the program hasn't asked for mouse events".into());
        }
        if m.line >= t.state.lines() || m.col >= t.state.cols() {
            return Err(format!("cell {},{} is off the screen", m.line, m.col));
        }
        drop(t);
        let button = match m.button.as_deref().unwrap_or("left") {
            "left" => ms::LEFT,
            "middle" => ms::MIDDLE,
            "right" => ms::RIGHT,
            other => return Err(format!("button {other:?}: left, middle or right")),
        };
        let (mut shift, mut alt, mut ctrl) = (false, false, false);
        for part in m.mods.as_deref().unwrap_or("").split('+').filter(|p| !p.is_empty()) {
            match part {
                "shift" => shift = true,
                "alt" | "option" | "meta" => alt = true,
                "ctrl" | "control" => ctrl = true,
                other => return Err(format!("mouse modifier {other:?}: shift, alt or ctrl")),
            }
        }
        let mods = ms::modifier_bits(shift, alt, ctrl);
        let sgr = modes.mouse_sgr;
        let one =
            |code: u8, press: bool| ms::encode(code, m.line, m.col, press, sgr).ok_or("that cell is past what X10 mouse reports can carry");
        let mut out = Vec::new();
        match m.kind.as_str() {
            "click" => {
                out.extend(one(button + mods, true)?);
                out.extend(one(button + mods, false)?);
            }
            "press" => out.extend(one(button + mods, true)?),
            "release" => out.extend(one(button + mods, false)?),
            "move" => {
                if !ms::reports_motion(modes.mouse, m.button.as_ref().map(|_| button)) {
                    return Err("the program doesn't ask for those moves".into());
                }
                let held = m.button.as_ref().map(|_| button);
                out.extend(one(ms::motion_code(held, None, mods), true)?);
            }
            "scroll" => {
                let lines = m.lines.unwrap_or(1);
                for _ in 0..lines.unsigned_abs().min(ms::MAX_WHEEL_REPORTS as u32) {
                    out.extend(one(ms::wheel_code(lines as isize, mods), true)?);
                }
            }
            other => return Err(format!("mouse {other:?}: click, press, release, move or scroll")),
        }
        Ok(out)
    }

    pub fn screen(&self, scrollback: usize, json: bool) -> Value {
        super::screen::screen(&mut self.term().state, scrollback, json)
    }

    /// Wait until `w` holds or its timeout runs out.
    pub fn wait_for(&self, w: &WaitFor) -> Result<Value, String> {
        let given = [w.text.is_some(), w.regex.is_some(), w.idle_ms.is_some(), w.exit, w.command]
            .iter()
            .filter(|&&b| b)
            .count();
        if given != 1 {
            return Err("wait-for takes one of text, regex, idle_ms, exit or command".into());
        }
        if w.command && !self.integrated {
            return Err("wait-for --command needs a --shell session with shell_integration on: nothing else marks commands".into());
        }
        let regex = match &w.regex {
            Some(r) => Some(regex_lite::Regex::new(r).map_err(|e| format!("regex: {e}"))?),
            None => None,
        };
        let timeout = Duration::from_millis(w.timeout_ms.unwrap_or(super::protocol::DEFAULT_TIMEOUT_MS));
        let t0 = Instant::now();
        let until = t0 + timeout;
        let reply = |matched: bool, reason: &str, extra: Value| {
            let mut v = json!({ "ok": true, "matched": matched, "reason": reason, "elapsed_ms": t0.elapsed().as_millis() as u64 });
            if let (Some(o), Value::Object(e)) = (v.as_object_mut(), extra) {
                o.extend(e);
            }
            v
        };
        let mut c = lock(&self.shared.counts);
        // The next command to finish is the one running now, if any.
        let finished_before = c.commands.finished;
        loop {
            let exited = c.exit;
            let seen = c.batches;
            if let Some(needle) = &w.text {
                drop(c);
                let lines = super::screen::lines(&mut self.term().state, 0);
                if let Some((row, col)) = lines
                    .iter()
                    .enumerate()
                    .find_map(|(i, l)| l.find(needle.as_str()).map(|b| (i, l[..b].chars().count())))
                {
                    return Ok(reply(true, "text", json!({ "row": row, "col": col })));
                }
                c = lock(&self.shared.counts);
                if c.batches != seen || c.exit.is_some() != exited.is_some() {
                    continue;
                }
            } else if let Some(re) = &regex {
                drop(c);
                let text = super::screen::lines(&mut self.term().state, 0).join("\n");
                if let Some(m) = re.find(&text) {
                    let row = text[..m.start()].matches('\n').count();
                    return Ok(reply(true, "regex", json!({ "row": row, "match": m.as_str() })));
                }
                c = lock(&self.shared.counts);
                if c.batches != seen || c.exit.is_some() != exited.is_some() {
                    continue;
                }
            } else if w.exit
                && let Some(e) = exited
            {
                return Ok(reply(true, "exit", json!({ "exit": exit_json(Some(e)) })));
            } else if w.command
                && let Some(done) = c.commands.after(finished_before)
            {
                return Ok(reply(true, "command", command_json(done)));
            }
            let now = Instant::now();
            if let Some(ms) = w.idle_ms {
                let quiet = Duration::from_millis(ms);
                let since = c.last_output.unwrap_or(self.started);
                if now >= since + quiet {
                    return Ok(reply(true, "idle", json!({})));
                }
                if now >= until {
                    return Ok(reply(false, "timeout", json!({})));
                }
                let wake = (since + quiet).min(until);
                c = self
                    .shared
                    .changed
                    .wait_timeout(c, wake - now)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
                continue;
            }
            if exited.is_some() && !w.exit {
                // Its last output was looked at above.
                return Ok(reply(false, "exited", json!({ "exit": exit_json(exited) })));
            }
            if now >= until {
                return Ok(reply(false, "timeout", json!({})));
            }
            c = self
                .shared
                .changed
                .wait_timeout(c, until - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<Value, String> {
        if cols == 0 || rows == 0 || cols > 1000 || rows > 1000 {
            return Err(format!("size {cols}x{rows}: 1 to 1000 each"));
        }
        let size = self.look.size(cols, rows);
        // The window is told outside the lock: it answers from the main
        // thread.
        let window = match &*lock(&self.link) {
            Some(Link::Own(s)) => {
                s.resize(size).map_err(|e| format!("resize: {e}"))?;
                None
            }
            Some(Link::Window { control, .. }) => Some(control.clone()),
            None => None,
        };
        if let Some(control) = window {
            control.resize(cols, rows)?;
        }
        if let Some(r) = &mut lock(&self.shared.counts).recorder {
            r.event("r", &format!("{cols}x{rows}"));
        }
        Ok(json!({ "ok": true, "cols": cols, "rows": rows }))
    }

    pub fn record(&self, path: Option<&str>, input: bool) -> Result<Value, String> {
        let mut c = lock(&self.shared.counts);
        let old = c.recorder.take().map(|r| r.path);
        if let Some(p) = path {
            let (cols, rows) = {
                let t = self.term();
                (t.state.cols() as u16, t.state.lines() as u16)
            };
            c.recorder = Some(Recorder::start(p, cols, rows, &self.term_name, input)?);
        }
        Ok(json!({ "ok": true, "recording": path, "stopped": old }))
    }

    pub fn set_shot(&self, times: ShotTimes) {
        lock(&self.shared.counts).shot = Some(times);
    }

    pub fn stats(&self) -> Value {
        let c = lock(&self.shared.counts);
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let history_bytes = self.term().state.scrollback_bytes();
        json!({
            "ok": true,
            "session": self.name,
            "pid": self.pid,
            "window": self.window,
            "running": c.exit.is_none(),
            "exit": exit_json(c.exit),
            "uptime_ms": self.started.elapsed().as_millis() as u64,
            "spawn_ms": ms(self.spawn_time),
            "first_output_ms": c.first_output.map(ms),
            "bytes_in": c.bytes_in,
            "bytes_out": c.bytes_out,
            "batches": c.batches,
            "parse_ms": ms(c.parse),
            "parse_max_ms": ms(c.parse_max),
            "parse_mb_per_s": if c.parse.is_zero() { 0.0 } else { c.bytes_in as f64 / c.parse.as_secs_f64() / 1e6 },
            "history_bytes": history_bytes,
            "recording": c.recorder.as_ref().map(|r| r.path.clone()),
            "screenshot": c.shot.map(|s| json!({
                "build_ms": ms(s.build), "gpu_ms": ms(s.gpu), "total_ms": ms(s.total), "width": s.width, "height": s.height,
            })),
        })
    }

    pub fn summary(&self) -> Value {
        let c = lock(&self.shared.counts);
        let t = self.term();
        json!({
            "session": self.name,
            "argv": self.argv,
            "pid": self.pid,
            "window": self.window,
            "running": c.exit.is_none(),
            "exit": exit_json(c.exit),
            "cols": t.state.cols(),
            "rows": t.state.lines(),
            "title": t.state.title(),
        })
    }

    /// Send `signal`, or with none hang up and let the session go.
    pub fn kill(&self, signal: Option<&str>) -> Result<Value, String> {
        if let Some(name) = signal {
            let sig = signal_number(name).ok_or_else(|| format!("signal {name:?}: try INT, TERM, KILL, HUP"))?;
            if let Some(c) = &self.child {
                c.signal(sig, true);
            }
            return Ok(json!({ "ok": true, "signal": sig }));
        }
        self.hang_up();
        // A moment for the status, which most programs give at once.
        let mut c = lock(&self.shared.counts);
        let until = Instant::now() + Duration::from_millis(500);
        while c.exit.is_none() {
            let now = Instant::now();
            if now >= until {
                break;
            }
            c = self
                .shared
                .changed
                .wait_timeout(c, until - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        Ok(json!({ "ok": true, "exit": exit_json(c.exit) }))
    }
}

impl Headless {
    /// Hang up on the program: an own session's, or by closing the
    /// window it is shown in.
    fn hang_up(&self) {
        let link = lock(&self.link).take();
        match link {
            Some(Link::Own(s)) => drop(s.hang_up()),
            Some(Link::Window { control, .. }) => control.close(),
            None => {}
        }
    }
}

impl Drop for Headless {
    fn drop(&mut self) {
        self.hang_up();
    }
}

/// `wait-for --command`'s result: how the command ended.
fn command_json(d: &Done) -> Value {
    json!({ "status": d.status, "duration_ms": d.duration.as_millis() as u64, "command": d.command })
}

fn exit_json(e: Option<Exit>) -> Value {
    match e {
        None => Value::Null,
        Some(e) => json!({ "code": e.code, "signal": e.signal, "after_ms": e.after.as_millis() as u64 }),
    }
}

fn signal_number(name: &str) -> Option<i32> {
    if let Ok(n) = name.parse::<i32>() {
        return (1..32).contains(&n).then_some(n);
    }
    let n = name.trim_start_matches("SIG").trim_start_matches("sig").to_ascii_uppercase();
    Some(match n.as_str() {
        "HUP" => libc::SIGHUP,
        "INT" => libc::SIGINT,
        "QUIT" => libc::SIGQUIT,
        "KILL" => libc::SIGKILL,
        "TERM" => libc::SIGTERM,
        "USR1" => libc::SIGUSR1,
        "USR2" => libc::SIGUSR2,
        "STOP" => libc::SIGSTOP,
        "CONT" => libc::SIGCONT,
        "WINCH" => libc::SIGWINCH,
        "TSTP" => libc::SIGTSTP,
        _ => return None,
    })
}

/// `program` found on `path` (or as a path, relative to `cwd`).
fn which(program: &str, path: &str, cwd: Option<&str>) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let runnable = |p: &std::path::Path| p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    if program.contains('/') {
        let p = match cwd {
            Some(dir) if !program.starts_with('/') => std::path::Path::new(dir).join(program),
            _ => program.into(),
        };
        return runnable(&p).then(|| p.to_string_lossy().into_owned());
    }
    path.split(':')
        .filter(|d| !d.is_empty())
        .map(|d| std::path::Path::new(d).join(program))
        .find(|p| runnable(p))
        .map(|p| p.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_split_across_batches() {
        assert_eq!(utf8_prefix(b"abc"), 3);
        let e = "é".as_bytes();
        assert_eq!(utf8_prefix(&[b'a', e[0]]), 1);
        assert_eq!(utf8_prefix(&[b'a', e[0], e[1]]), 3);
        let check = "✓".as_bytes();
        assert_eq!(utf8_prefix(&check[..2]), 0);
        assert_eq!(utf8_prefix(check), 3);
    }

    #[test]
    fn commands_finish_with_status_and_duration() {
        use term_core::Command::{Finished, Prompt, Started};
        let t0 = Instant::now();
        let mut c = Commands::default();
        c.apply(&[Prompt], t0);
        assert!(c.after(0).is_none());
        // Running, not finished.
        c.apply(&[Started(Some("make".into()))], t0);
        assert!(c.after(0).is_none());
        c.apply(&[Finished(Some(2)), Prompt], t0 + Duration::from_millis(1500));
        assert_eq!(
            c.after(0),
            Some(&Done {
                status: Some(2),
                duration: Duration::from_millis(1500),
                command: Some("make".into())
            })
        );
        // A wait that began after it sees only later ones.
        assert!(c.after(1).is_none());
        c.apply(&[Started(None), Finished(None)], t0 + Duration::from_secs(3));
        let d = c.after(1).unwrap();
        assert_eq!((d.status, d.duration, d.command.clone()), (None, Duration::ZERO, None));
        // Only the last few are kept, and a wait still finds the oldest after it.
        for i in 0..DONE_KEPT as u32 + 2 {
            c.apply(&[Started(None), Finished(Some(i))], t0);
        }
        assert_eq!(c.done.len(), DONE_KEPT);
        assert_eq!(c.after(c.finished - 1).map(|d| d.status), Some(Some(DONE_KEPT as u32 + 1)));
    }

    #[test]
    fn finds_programs() {
        assert_eq!(which("sh", "/nope:/bin", None).as_deref(), Some("/bin/sh"));
        assert_eq!(which("/bin/sh", "", None).as_deref(), Some("/bin/sh"));
        assert_eq!(which("sh", "", Some("/bin")), None);
        assert_eq!(which("./sh", "", Some("/bin")).as_deref(), Some("/bin/./sh"));
        assert_eq!(which("no-such-program-here", "/bin:/usr/bin", None), None);
        assert_eq!(signal_number("SIGINT"), Some(libc::SIGINT));
        assert_eq!(signal_number("term"), Some(libc::SIGTERM));
        assert_eq!(signal_number("9"), Some(9));
        assert_eq!(signal_number("nope"), None);
    }
}
