//! Where the completion list's choices come from, and whether a command
//! exists (docs/design/completion.md §6). Programs on PATH and folder
//! listings are read on one worker thread for the whole process, never
//! on the main thread or a PTY reader; the main thread only looks in the
//! cache, and asks for what isn't there. The worker exits after a minute
//! with nothing asked, its caches with it, so a terminal nobody types
//! in holds nothing.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

use term_core::ShellEnv;

/// Called on the worker when something asked for is ready: the pane
/// that asked draws again.
pub type Wake = Box<dyn FnOnce() + Send>;

/// How long the worker waits for a job before exiting.
const IDLE_EXIT: Duration = Duration::from_secs(60);
/// A cached listing is checked against the disk again at most this often.
const RECHECK: Duration = Duration::from_secs(2);
/// Folders kept, oldest dropped first.
const FOLDERS_KEPT: usize = 64;
/// Entries kept per folder: a huge one is cut short, not held whole.
const FOLDER_ENTRIES: usize = 10_000;

/// The programs on a PATH.
#[derive(Debug, Default)]
pub struct Commands {
    /// Sorted, without duplicates.
    pub names: Vec<String>,
    set: HashSet<String>,
}

impl Commands {
    pub fn contains(&self, name: &str) -> bool {
        self.set.contains(name)
    }

    /// From names in any order, duplicates allowed.
    pub fn from_names(names: Vec<String>) -> Commands {
        Commands::new(names)
    }

    fn new(mut names: Vec<String>) -> Commands {
        names.sort_unstable();
        names.dedup();
        let set = names.iter().cloned().collect();
        Commands { names, set }
    }
}

/// A file or folder in a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub executable: bool,
}

/// A folder's entries, sorted by name.
#[derive(Debug, Default)]
pub struct Folder {
    pub entries: Vec<Entry>,
}

impl Folder {
    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries
            .binary_search_by(|e| e.name.as_str().cmp(name))
            .ok()
            .map(|i| &self.entries[i])
    }
}

enum Job {
    Path(String),
    Folder(PathBuf),
}

/// A cached value, when it was last checked, and the wake-ups waiting
/// for it.
struct Slot<T> {
    value: Option<Arc<T>>,
    /// The mtimes it was read at: the PATH's folders, or the folder.
    mtimes: Vec<Option<SystemTime>>,
    checked: Option<Instant>,
    /// Asked for and not read yet.
    pending: bool,
    waiting: Vec<Wake>,
}

impl<T> Default for Slot<T> {
    fn default() -> Slot<T> {
        Slot {
            value: None,
            mtimes: Vec::new(),
            checked: None,
            pending: false,
            waiting: Vec::new(),
        }
    }
}

impl<T> Slot<T> {
    /// What is cached, asking for it (again) when it is missing or due a
    /// check; true in `.1` when a job should be sent.
    fn look(&mut self, wake: impl FnOnce() -> Wake, now: Instant) -> (Option<Arc<T>>, bool) {
        let due = self.checked.is_none_or(|t| now.duration_since(t) >= RECHECK);
        let send = due && !self.pending;
        if send {
            self.pending = true;
        }
        if self.value.is_none() {
            self.waiting.push(wake());
        }
        (self.value.clone(), send)
    }
}

#[derive(Default)]
struct State {
    tx: Option<Sender<Job>>,
    paths: HashMap<String, Slot<Commands>>,
    folders: HashMap<PathBuf, Slot<Folder>>,
    /// Folders oldest first, for dropping.
    folder_order: VecDeque<PathBuf>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn state() -> MutexGuard<'static, Option<State>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Send `job`, starting the worker if it isn't running.
fn send(st: &mut State, job: Job) {
    let job = match &st.tx {
        Some(tx) => match tx.send(job) {
            Ok(()) => return,
            Err(e) => e.0,
        },
        None => job,
    };
    let (tx, rx) = channel();
    let _ = tx.send(job);
    st.tx = Some(tx);
    let _ = std::thread::Builder::new().name("completion".into()).spawn(move || worker(rx));
}

/// The programs on `path` (a `$PATH` string), if read; asks for them
/// otherwise, and `wake` is called when they are ready.
pub fn commands(path: &str, wake: impl FnOnce() -> Wake) -> Option<Arc<Commands>> {
    let mut guard = state();
    let st = guard.get_or_insert_with(State::default);
    let (value, ask) = st.paths.entry(path.to_string()).or_default().look(wake, Instant::now());
    if ask {
        send(st, Job::Path(path.to_string()));
    }
    value
}

/// The entries of folder `dir` (absolute), if read; asks for them
/// otherwise, as [`commands`].
pub fn folder(dir: &Path, wake: impl FnOnce() -> Wake) -> Option<Arc<Folder>> {
    let mut guard = state();
    let st = guard.get_or_insert_with(State::default);
    if !st.folders.contains_key(dir) {
        st.folder_order.push_back(dir.to_path_buf());
        while st.folder_order.len() > FOLDERS_KEPT {
            if let Some(old) = st.folder_order.pop_front() {
                st.folders.remove(&old);
            }
        }
    }
    let (value, ask) = st.folders.entry(dir.to_path_buf()).or_default().look(wake, Instant::now());
    if ask {
        send(st, Job::Folder(dir.to_path_buf()));
    }
    value
}

fn worker(rx: Receiver<Job>) {
    loop {
        let job = match rx.recv_timeout(IDLE_EXIT) {
            Ok(job) => job,
            Err(RecvTimeoutError::Timeout) => {
                let mut guard = state();
                // A job may have come in while the lock was being taken.
                if let Ok(job) = rx.try_recv() {
                    drop(guard);
                    run(job);
                    continue;
                }
                // Nothing asked for a minute: give everything back.
                *guard = None;
                return;
            }
            Err(RecvTimeoutError::Disconnected) => return,
        };
        run(job);
    }
}

fn run(job: Job) {
    match job {
        Job::Path(path) => {
            let dirs: Vec<PathBuf> = std::env::split_paths(&path).filter(|d| d.is_absolute()).collect();
            let mtimes: Vec<_> = dirs.iter().map(|d| mtime(d)).collect();
            let fresh = with_slot_paths(&path, |s| s.value.is_some() && s.mtimes == mtimes);
            let value = (!fresh).then(|| Arc::new(Commands::new(dirs.iter().flat_map(|d| executables(d)).collect())));
            finish(|st| st.paths.get_mut(&path), value, mtimes);
        }
        Job::Folder(dir) => {
            let mtimes = vec![mtime(&dir)];
            let fresh = with_slot_folders(&dir, |s| s.value.is_some() && s.mtimes == mtimes);
            let value = (!fresh).then(|| Arc::new(read_folder(&dir)));
            finish(|st| st.folders.get_mut(&dir), value, mtimes);
        }
    }
}

fn with_slot_paths(path: &str, f: impl FnOnce(&Slot<Commands>) -> bool) -> bool {
    state().as_ref().and_then(|st| st.paths.get(path)).is_some_and(f)
}

fn with_slot_folders(dir: &Path, f: impl FnOnce(&Slot<Folder>) -> bool) -> bool {
    state().as_ref().and_then(|st| st.folders.get(dir)).is_some_and(f)
}

/// Store what was read (`None`: unchanged) and wake whoever waited.
fn finish<T>(slot: impl FnOnce(&mut State) -> Option<&mut Slot<T>>, value: Option<Arc<T>>, mtimes: Vec<Option<SystemTime>>) {
    let waiting = {
        let mut guard = state();
        let Some(s) = guard.as_mut().and_then(slot) else { return };
        if let Some(v) = value {
            s.value = Some(v);
            s.mtimes = mtimes;
        }
        s.checked = Some(Instant::now());
        s.pending = false;
        std::mem::take(&mut s.waiting)
    };
    for wake in waiting {
        wake();
    }
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The executable files (and links to them) in `dir`.
fn executables(dir: &Path) -> Vec<String> {
    use std::os::unix::fs::PermissionsExt;
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten()
        .filter(|e| {
            // Follows links, as running it would.
            std::fs::metadata(e.path()).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

fn read_folder(dir: &Path) -> Folder {
    use std::os::unix::fs::PermissionsExt;
    let Ok(rd) = std::fs::read_dir(dir) else { return Folder::default() };
    let mut entries: Vec<Entry> = rd
        .flatten()
        .take(FOLDER_ENTRIES)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            // Links followed: a link to a folder is a folder.
            let m = std::fs::metadata(e.path()).ok();
            Some(Entry {
                name,
                dir: m.as_ref().is_some_and(|m| m.is_dir()),
                executable: m.as_ref().is_some_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0),
            })
        })
        .collect();
    entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));
    Folder { entries }
}

/// Whether `name`, typed where a command goes, runs something: `Some`
/// once that is certain either way, `None` while what decides it is
/// still being read (or can't be known: an expansion). `cwd` is the
/// shell's folder, for `./x` and `autocd`.
pub fn command_known(name: &str, env: &ShellEnv, cwd: Option<&str>, wake: impl Fn() -> Wake) -> Option<bool> {
    if name.is_empty() || name.contains('$') || name.contains('`') {
        return None;
    }
    let listed = |list: &[String]| list.iter().any(|n| n == name);
    if listed(&env.aliases) || listed(&env.global_aliases) || listed(&env.functions) || listed(&env.builtins) || listed(&env.reserved_words)
    {
        return Some(true);
    }
    let in_folder = |path: &Path| -> Option<Option<Entry>> {
        let parent = path.parent()?;
        let file = path.file_name()?.to_str()?;
        let f = folder(parent, &wake)?;
        Some(f.get(file).cloned())
    };
    if name.contains('/') {
        // A path: an executable file there (or a folder, with autocd).
        let path = resolve(name, cwd)?;
        let e = in_folder(&path)?;
        return Some(e.is_some_and(|e| e.executable || (e.dir && env.autocd)));
    }
    let on_path = commands(&env.path, &wake)?.contains(name);
    if on_path {
        return Some(true);
    }
    if env.autocd {
        // A folder in the current one, run as `cd`.
        let path = resolve(name, cwd)?;
        let e = in_folder(&path)?;
        return Some(e.is_some_and(|e| e.dir));
    }
    Some(false)
}

/// `name` as an absolute path: `~` is the home folder, relative ones are
/// in `cwd`. `None` without a folder to resolve against.
fn resolve(name: &str, cwd: Option<&str>) -> Option<PathBuf> {
    if let Some(rest) = name.strip_prefix("~/") {
        return Some(PathBuf::from(std::env::var_os("HOME")?).join(rest));
    }
    let p = Path::new(name);
    if p.is_absolute() {
        return Some(p.to_path_buf());
    }
    Some(Path::new(cwd?).join(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Wait for what `f` returns to be `Some`, asking again after each
    /// wake-up; the wake-ups counted in `woken`.
    fn ready<T>(woken: &Arc<AtomicUsize>, f: impl Fn(&dyn Fn() -> Wake) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let w = woken.clone();
            let wake = move || -> Wake {
                let w = w.clone();
                Box::new(move || {
                    w.fetch_add(1, Ordering::SeqCst);
                })
            };
            if let Some(v) = f(&wake) {
                return v;
            }
            assert!(Instant::now() < deadline, "never ready");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("decline-sources-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn file(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn programs_on_path_and_changes_to_them() {
        let (a, b) = (temp("path-a"), temp("path-b"));
        file(&a.join("tool"), 0o755);
        file(&a.join("notes.txt"), 0o644);
        file(&b.join("other"), 0o755);
        std::fs::create_dir(b.join("subdir")).unwrap();
        let path = format!("{}:{}:relative", a.display(), b.display());
        let woken = Arc::new(AtomicUsize::new(0));
        let c = ready(&woken, |w| commands(&path, w));
        assert_eq!(c.names, ["other", "tool"]);
        assert!(woken.load(Ordering::SeqCst) >= 1, "the first ask is woken when ready");
        // A new program shows after the next check.
        std::thread::sleep(Duration::from_millis(1100));
        file(&a.join("newer"), 0o755);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let c = ready(&woken, |w| commands(&path, w));
            if c.contains("newer") {
                break;
            }
            assert!(Instant::now() < deadline, "the new program never showed: {:?}", c.names);
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    #[test]
    fn whether_a_command_runs_something() {
        let bin = temp("known-bin");
        let cwd = temp("known-cwd");
        file(&bin.join("make"), 0o755);
        file(&cwd.join("script"), 0o755);
        file(&cwd.join("data"), 0o644);
        std::fs::create_dir(cwd.join("project")).unwrap();
        let env = ShellEnv {
            path: bin.display().to_string(),
            aliases: vec!["ll".into()],
            builtins: vec!["cd".into()],
            functions: vec!["myfn".into()],
            reserved_words: vec!["if".into()],
            ..ShellEnv::default()
        };
        let cwd_s = cwd.display().to_string();
        let woken = Arc::new(AtomicUsize::new(0));
        let known = |name: &str, env: &ShellEnv| ready(&woken, |w| command_known(name, env, Some(&cwd_s), w));
        for (name, want) in [
            ("ll", true),
            ("cd", true),
            ("myfn", true),
            ("if", true),
            ("make", true),
            ("mak", false),
            ("./script", true),
            ("./data", false),
            ("./missing", false),
            ("project", false),
        ] {
            assert_eq!(known(name, &env), want, "{name}");
        }
        let autocd = ShellEnv {
            autocd: true,
            ..env.clone()
        };
        assert!(known("project", &autocd));
        assert!(known("./project", &autocd));
        assert!(!known("data", &autocd));
        // An expansion can't be known.
        assert_eq!(command_known("$EDITOR", &env, Some(&cwd_s), || Box::new(|| {})), None);
    }
}
