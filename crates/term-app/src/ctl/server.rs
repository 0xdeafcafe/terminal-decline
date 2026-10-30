//! The agent API's server: the socket, who may connect, and the sessions
//! (docs/design/agent-api.md §2–§3). Served by the app or by the
//! headless server; each connection has a thread of its own.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::headless::{Headless, Look};
use super::protocol::{self, Request};

/// Who serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    App,
    Headless,
}

/// How long a request line may be.
const MAX_LINE: usize = 16 << 20;

pub struct Server {
    kind: Kind,
    dir: PathBuf,
    source: crate::config::Source,
    sessions: Mutex<BTreeMap<String, Arc<Headless>>>,
    next: AtomicU64,
    connections: AtomicUsize,
    last_request: Mutex<Instant>,
    /// Held while serving: one server per directory.
    _lock: File,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Take `dir`'s lock and listen on its socket. `None`: another server
/// holds the lock and serves.
pub fn bind(dir: &Path) -> io::Result<Option<(UnixListener, File)>> {
    protocol::ensure_dir(dir)?;
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("ctl.lock"))?;
    if unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(None);
    }
    let path = protocol::socket_path(dir);
    // Nobody holds the lock: a socket left there is stale.
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    // Made 0600 before anyone could connect: the directory is ours alone.
    let listener = UnixListener::bind(&path)?;
    protocol::restrict_socket(&path)?;
    Ok(Some((listener, lock_file)))
}

impl Server {
    pub fn new(kind: Kind, dir: &Path, source: crate::config::Source, lock_file: File) -> Arc<Server> {
        Arc::new(Server {
            kind,
            dir: dir.to_path_buf(),
            source,
            sessions: Mutex::default(),
            next: AtomicU64::new(1),
            connections: AtomicUsize::new(0),
            last_request: Mutex::new(Instant::now()),
            _lock: lock_file,
        })
    }

    /// Accept connections until the listener fails.
    pub fn serve(self: &Arc<Server>, listener: UnixListener) {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            if !protocol::same_user(&stream) {
                // Another user: nothing is read or answered.
                continue;
            }
            let server = self.clone();
            server.connections.fetch_add(1, Ordering::AcqRel);
            let spawned = std::thread::Builder::new().name("ctl-conn".into()).spawn(move || {
                let _ = server.connection(stream);
                *lock(&server.last_request) = Instant::now();
                server.connections.fetch_sub(1, Ordering::AcqRel);
            });
            if spawned.is_err() {
                self.connections.fetch_sub(1, Ordering::AcqRel);
            }
        }
    }

    /// The headless server: exit once nothing runs, nobody is connected
    /// and nothing has asked for `idle`.
    pub fn exit_when_idle(self: &Arc<Server>, idle: Duration) {
        let server = self.clone();
        let _ = std::thread::Builder::new().name("ctl-idle".into()).spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(200).min(idle));
                let busy = server.connections.load(Ordering::Acquire) > 0 || lock(&server.sessions).values().any(|s| s.running());
                if busy {
                    *lock(&server.last_request) = Instant::now();
                    continue;
                }
                if lock(&server.last_request).elapsed() >= idle {
                    let _ = std::fs::remove_file(protocol::socket_path(&server.dir));
                    std::process::exit(0);
                }
            }
        });
    }

    fn connection(&self, stream: UnixStream) -> io::Result<()> {
        let mut out = stream.try_clone()?;
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        loop {
            line.clear();
            let n = io::Read::take(&mut reader, MAX_LINE as u64).read_line(&mut line)?;
            if n == 0 {
                return Ok(());
            }
            if line.trim().is_empty() {
                continue;
            }
            *lock(&self.last_request) = Instant::now();
            let reply = match serde_json::from_str::<Request>(&line) {
                Ok(req) => self.dispatch(req),
                Err(e) => json!({ "ok": false, "error": format!("bad request: {e}") }),
            };
            // One write: Display would make one per token.
            let mut text = reply.to_string();
            text.push('\n');
            out.write_all(text.as_bytes())?;
        }
    }

    fn session(&self, name: &str) -> Result<Arc<Headless>, String> {
        lock(&self.sessions)
            .get(name)
            .cloned()
            .ok_or_else(|| format!("no session {name:?}"))
    }

    /// Answer one request.
    pub fn dispatch(&self, req: Request) -> Value {
        match self.answer(req) {
            Ok(v) => v,
            Err(e) => json!({ "ok": false, "error": e }),
        }
    }

    fn answer(&self, req: Request) -> Result<Value, String> {
        match req {
            Request::Hello => Ok(json!({
                "ok": true,
                "server": match self.kind { Kind::App => "app", Kind::Headless => "headless" },
                "pid": std::process::id(),
                "version": env!("CARGO_PKG_VERSION"),
                "socket": protocol::socket_path(&self.dir),
            })),
            Request::Spawn(spawn) => {
                if spawn.window {
                    return Err("--window isn't there yet: sessions are headless (docs/design/agent-api.md §6)".into());
                }
                let name = match &spawn.name {
                    Some(n) if n.is_empty() || n.len() > 64 || n.contains(char::is_whitespace) => {
                        return Err(format!("session name {n:?}: 1–64 characters, no spaces"));
                    }
                    Some(n) => n.clone(),
                    None => loop {
                        let n = self.next.fetch_add(1, Ordering::Relaxed).to_string();
                        if !lock(&self.sessions).contains_key(&n) {
                            break n;
                        }
                    },
                };
                if lock(&self.sessions).contains_key(&name) {
                    return Err(format!("a session {name:?} exists already (kill it first)"));
                }
                let (config, _notices) = self.source.load();
                if !config.agent_api {
                    return Err("the agent API is off (agent_api = false)".into());
                }
                let look = Look::new(config, &spawn, &self.source.themes_dir)?;
                let t0 = Instant::now();
                let h = Arc::new(Headless::spawn(name.clone(), &spawn, look)?);
                let took = t0.elapsed();
                let pid = h.summary()["pid"].clone();
                let mut sessions = lock(&self.sessions);
                if sessions.contains_key(&name) {
                    return Err(format!("a session {name:?} exists already (kill it first)"));
                }
                sessions.insert(name.clone(), h);
                Ok(json!({ "ok": true, "session": name, "pid": pid, "spawn_ms": took.as_secs_f64() * 1000.0 }))
            }
            Request::Send { session, actions } => self.session(&session)?.send(&actions),
            Request::Screen { session, json, scrollback } => Ok(self.session(&session)?.screen(scrollback, json)),
            Request::WaitFor(w) => self.session(&w.session)?.wait_for(&w),
            Request::Resize { session, cols, rows } => self.session(&session)?.resize(cols, rows),
            Request::Screenshot { session, path } => {
                if !Path::new(&path).is_absolute() {
                    return Err(format!("{path}: give an absolute path"));
                }
                let h = self.session(&session)?;
                let (img, times) = super::shot::capture(&h)?;
                let t0 = Instant::now();
                super::shot::write_png(&img, &path)?;
                let png_ms = t0.elapsed().as_secs_f64() * 1000.0;
                h.set_shot(times);
                Ok(json!({
                    "ok": true,
                    "path": path,
                    "width": img.width,
                    "height": img.height,
                    "build_ms": times.build.as_secs_f64() * 1000.0,
                    "gpu_ms": times.gpu.as_secs_f64() * 1000.0,
                    "png_ms": png_ms,
                }))
            }
            Request::Stats { session } => {
                let mut v = self.session(&session)?.stats();
                v["server"] = self.process_stats();
                Ok(v)
            }
            Request::Record { session, path, input } => {
                if path.as_deref().is_some_and(|p| !Path::new(p).is_absolute()) {
                    return Err("give the recording an absolute path".into());
                }
                self.session(&session)?.record(path.as_deref(), input)
            }
            Request::List => {
                let list: Vec<Value> = lock(&self.sessions).values().map(|s| s.summary()).collect();
                Ok(json!({ "ok": true, "sessions": list, "server": self.process_stats() }))
            }
            Request::Kill { session, signal } => {
                let h = self.session(&session)?;
                let reply = h.kill(signal.as_deref())?;
                if signal.is_none() {
                    lock(&self.sessions).remove(&session);
                }
                Ok(reply)
            }
        }
    }

    fn process_stats(&self) -> Value {
        let sessions = lock(&self.sessions);
        json!({
            "kind": match self.kind { Kind::App => "app", Kind::Headless => "headless" },
            "pid": std::process::id(),
            "sessions": sessions.len(),
            "running": sessions.values().filter(|s| s.running()).count(),
            "phys_footprint": crate::control::phys_footprint(),
        })
    }
}
