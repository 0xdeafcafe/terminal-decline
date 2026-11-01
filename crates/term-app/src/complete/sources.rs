//! Where the completion list's choices come from, and whether a command
//! exists (docs/design/completion.md §6). Programs on PATH, folder
//! listings and man pages are read on one worker thread for the whole process, never
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

use super::help;
use super::man::{self, Page};

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
/// The most of a man page read: a bigger one is cut short.
const PAGE_BYTES: usize = 4 << 20;
/// Enough of a page for its NAME line.
const PAGE_HEAD_BYTES: usize = 16 << 10;
/// Subcommands kept per command.
const SUBCOMMANDS: usize = 1000;
/// The most of a program's `--help` kept.
const HELP_BYTES: u64 = 256 << 10;
/// How long a program gets to print its `--help`.
const HELP_TIME: Duration = Duration::from_secs(2);

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

/// A command's man page, read.
#[derive(Debug, Default)]
pub struct Manual {
    /// It has one.
    pub found: bool,
    pub page: Page,
}

/// How often each command was run, from the shell's history file.
#[derive(Debug, Default, Clone)]
pub struct History {
    pub counts: HashMap<String, u32>,
    /// How far the file was read: only what is added after is read next.
    read_to: u64,
}

impl History {
    pub fn count(&self, name: &str) -> u32 {
        self.counts.get(name).copied().unwrap_or(0)
    }

    /// From counts, for tests.
    #[cfg(test)]
    pub fn from_counts<'a>(counts: impl IntoIterator<Item = (&'a str, u32)>) -> History {
        History {
            counts: counts.into_iter().map(|(n, c)| (n.to_string(), c)).collect(),
            read_to: 0,
        }
    }
}

/// A command's subcommands: the man pages named `<command>-<name>`.
#[derive(Debug, Default)]
pub struct Subcommands {
    /// Names and their pages' descriptions, sorted by name.
    pub list: Vec<(String, String)>,
}

enum Job {
    Path(String),
    Folder(PathBuf),
    /// A PATH and a command's (or `command-subcommand`'s) name.
    Man(String, String),
    Subcommands(String, String),
    /// Run a program's `--help` (asked for from the menu) and keep it.
    Help(Help),
    History(PathBuf),
}

/// A `--help` to read: the program, the subcommand before `--help`, the
/// PATH to run it with, and the name its output is kept under.
pub struct Help {
    pub exe: PathBuf,
    pub sub: Option<String>,
    pub path: String,
    pub name: String,
    /// Called with how many flags and subcommands were found.
    pub done: Box<dyn FnOnce(Result<usize, String>) + Send>,
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
    /// The man page folders for a PATH, as `manpath` says.
    manpaths: HashMap<String, Arc<Vec<PathBuf>>>,
    manuals: HashMap<(String, String), Slot<Manual>>,
    subcommands: HashMap<(String, String), Slot<Subcommands>>,
    histories: HashMap<PathBuf, Slot<History>>,
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

/// `name`'s man page on `path`'s man folders, if read; asks for it
/// otherwise, as [`commands`].
pub fn manual(path: &str, name: &str, wake: impl FnOnce() -> Wake) -> Option<Arc<Manual>> {
    let mut guard = state();
    let st = guard.get_or_insert_with(State::default);
    let key = (path.to_string(), name.to_string());
    let (value, ask) = st.manuals.entry(key.clone()).or_default().look(wake, Instant::now());
    if ask {
        send(st, Job::Man(key.0, key.1));
    }
    value
}

/// How often each command in history file `file` was run, if read; asks
/// for it otherwise, and again when the file changes.
pub fn history(file: &Path, wake: impl FnOnce() -> Wake) -> Option<Arc<History>> {
    let mut guard = state();
    let st = guard.get_or_insert_with(State::default);
    let (value, ask) = st.histories.entry(file.to_path_buf()).or_default().look(wake, Instant::now());
    if ask {
        send(st, Job::History(file.to_path_buf()));
    }
    value
}

/// Run a program's `--help` on the worker and keep what it prints, for
/// [`manual`] and [`subcommands`] to read where there is no man page.
pub fn read_help(job: Help) {
    let mut guard = state();
    let st = guard.get_or_insert_with(State::default);
    send(st, Job::Help(job));
}

/// `command`'s subcommands, if read; asks for them otherwise.
pub fn subcommands(path: &str, command: &str, wake: impl FnOnce() -> Wake) -> Option<Arc<Subcommands>> {
    let mut guard = state();
    let st = guard.get_or_insert_with(State::default);
    let key = (path.to_string(), command.to_string());
    let (value, ask) = st.subcommands.entry(key.clone()).or_default().look(wake, Instant::now());
    if ask {
        send(st, Job::Subcommands(key.0, key.1));
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
        Job::Man(path, name) => {
            let file = find_page(&manpath(&path), &name);
            let kept = help_file(&name);
            let mtimes = vec![file.as_deref().and_then(mtime), kept.as_deref().and_then(mtime)];
            let key = (path, name);
            let fresh = state()
                .as_ref()
                .and_then(|st| st.manuals.get(&key))
                .is_some_and(|s| s.value.is_some() && s.mtimes == mtimes);
            let value = (!fresh).then(|| {
                let page = file.as_deref().and_then(|f| read_page(f, PAGE_BYTES)).map(|src| man::read(&src));
                // Without a man page: its `--help`, if one was read.
                let page = page.or_else(|| kept_help(&key.1).map(|h| h.page));
                Arc::new(Manual {
                    found: page.is_some(),
                    page: page.unwrap_or_default(),
                })
            });
            finish(|st| st.manuals.get_mut(&key), value, mtimes);
        }
        Job::Subcommands(path, command) => {
            let sections: Vec<PathBuf> = manpath(&path).iter().flat_map(|d| ["man1", "man8"].map(|s| d.join(s))).collect();
            let mut mtimes: Vec<_> = sections.iter().map(|d| mtime(d)).collect();
            mtimes.push(help_file(&command).as_deref().and_then(mtime));
            let key = (path, command);
            let fresh = state()
                .as_ref()
                .and_then(|st| st.subcommands.get(&key))
                .is_some_and(|s| s.value.is_some() && s.mtimes == mtimes);
            let value = (!fresh).then(|| {
                let mut subs = read_subcommands(&sections, &key.1);
                if subs.list.is_empty()
                    && let Some(h) = kept_help(&key.1)
                {
                    subs.list = h.subcommands;
                }
                Arc::new(subs)
            });
            finish(|st| st.subcommands.get_mut(&key), value, mtimes);
        }
        Job::Help(job) => run_help(job),
        Job::History(file) => {
            let mtimes = vec![mtime(&file)];
            // Unchanged: nothing read; grown: only what was added.
            let (fresh, old) = state()
                .as_ref()
                .and_then(|st| st.histories.get(&file))
                .map_or((false, None), |s| (s.value.is_some() && s.mtimes == mtimes, s.value.clone()));
            let value = (!fresh).then(|| Arc::new(read_history(&file, old.as_deref())));
            finish(|st| st.histories.get_mut(&file), value, mtimes);
        }
    }
}

/// The most of a history file read at once.
const HISTORY_BYTES: u64 = 8 << 20;

/// Count the commands in zsh history file `file`, carrying on from `old`
/// when the file only grew.
fn read_history(file: &Path, old: Option<&History>) -> History {
    use std::io::{Read, Seek, SeekFrom};
    let len = std::fs::metadata(file).map_or(0, |m| m.len());
    let mut h = match old {
        Some(o) if o.read_to <= len => o.clone(),
        _ => History::default(),
    };
    let Ok(mut f) = std::fs::File::open(file) else { return h };
    // Too big: its end only.
    let from = h.read_to.max(len.saturating_sub(HISTORY_BYTES));
    let mut bytes = Vec::new();
    if f.seek(SeekFrom::Start(from)).is_err() || f.take(HISTORY_BYTES).read_to_end(&mut bytes).is_err() {
        return h;
    }
    // Up to the last whole line; the rest is read next time.
    let end = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    count_history(&unmetafy(&bytes[..end]), &mut h.counts);
    h.read_to = from + end as u64;
    h
}

/// zsh's history file bytes as written: a byte after Meta (0x83) is
/// stored XOR 32.
fn unmetafy(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(bytes.len());
    let mut it = bytes.iter();
    while let Some(&b) = it.next() {
        if b == 0x83 {
            if let Some(&n) = it.next() {
                out.push(n ^ 32);
            }
        } else {
            out.push(b);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Add each command run in `text` (zsh history: `: <time>:<secs>;<line>`
/// or plain lines, a line ending in `\` continued on the next) to
/// `counts`.
fn count_history(text: &str, counts: &mut HashMap<String, u32>) {
    let mut entry = String::new();
    for line in text.lines() {
        let line = if entry.is_empty() {
            match line.strip_prefix(": ").and_then(|l| l.split_once(';')) {
                Some((_, cmd)) => cmd,
                None => line,
            }
        } else {
            line
        };
        if let Some(head) = line.strip_suffix('\\') {
            entry.push_str(head);
            entry.push('\n');
            continue;
        }
        entry.push_str(line);
        let read = super::lex::read(&entry, entry.len());
        for w in read.words.iter().filter(|w| w.role == super::lex::Role::Command && !w.expands) {
            *counts.entry(w.text.clone()).or_default() += 1;
        }
        entry.clear();
    }
}

fn run_help(job: Help) {
    let shown = job.name.replace('-', " ");
    let result = match help_output(&job) {
        None => Err(format!("{shown} --help printed nothing")),
        Some(text) => {
            let read = help::read(&text);
            let found = read.page.flags.len() + read.subcommands.len();
            match help_file(&job.name) {
                _ if found == 0 => Err(format!("found no flags in {shown} --help")),
                None => Err("no home folder to keep it in".into()),
                Some(file) => {
                    let _ = file.parent().map(std::fs::create_dir_all);
                    match std::fs::write(&file, &text) {
                        Ok(()) => Ok(found),
                        Err(e) => Err(format!("couldn't keep {}: {e}", file.display())),
                    }
                }
            }
        }
    };
    // What was read for it before is read again.
    if let Some(st) = state().as_mut() {
        st.manuals.retain(|(_, n), _| *n != job.name);
        st.subcommands.retain(|(_, n), _| *n != job.name);
    }
    (job.done)(result);
}

/// Run `exe [sub] --help`: with the shell's PATH, pagers as `cat`, no
/// colour, in an empty folder, stdin empty; its whole process group
/// killed after [`HELP_TIME`]. What it printed (stdout, else stderr), at
/// most [`HELP_BYTES`], with escapes and overstrikes taken out.
fn help_output(job: &Help) -> Option<String> {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let dir = std::env::temp_dir().join(format!("decline-help-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let mut cmd = Command::new(&job.exe);
    cmd.args(job.sub.iter())
        .arg("--help")
        .current_dir(&dir)
        .env("PATH", &job.path)
        .env("PAGER", "cat")
        .env("GIT_PAGER", "cat")
        .env("MANPAGER", "cat")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = cmd.spawn().ok()?;
    let group = child.id() as i32;
    fn reader(r: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(r) = r {
                let _ = r.take(HELP_BYTES).read_to_end(&mut buf);
            }
            buf
        })
    }
    let out = reader(child.stdout.take());
    let err = reader(child.stderr.take());
    let deadline = Instant::now() + HELP_TIME;
    while child.try_wait().ok()?.is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    // Done or not: nothing it started is left holding the pipes.
    // SAFETY: signals the process group this child leads.
    unsafe { libc::kill(-group, libc::SIGKILL) };
    let _ = child.wait();
    let (out, err) = (out.join().ok()?, err.join().ok()?);
    let bytes = if out.iter().any(|b| !b.is_ascii_whitespace()) { out } else { err };
    let text = clean(&String::from_utf8_lossy(&bytes));
    (!text.trim().is_empty()).then_some(text)
}

/// Text with escape sequences and backspace overstrikes taken out.
fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => {
                if chars.next() == Some('[') {
                    while chars.next().is_some_and(|c| !c.is_ascii_alphabetic()) {}
                }
            }
            '\x08' => {
                out.pop();
            }
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Where a `--help` read for `name` is kept: `~/Library/Caches/
/// terminal-decline/help/<name>.txt`.
fn help_file(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        return None;
    }
    #[cfg(test)]
    if let Some(dir) = HELP_DIR.lock().unwrap().clone() {
        return Some(dir.join(format!("{name}.txt")));
    }
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join(format!("Library/Caches/terminal-decline/help/{name}.txt")))
}

#[cfg(test)]
static HELP_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

fn kept_help(name: &str) -> Option<help::Help> {
    let text = std::fs::read_to_string(help_file(name)?).ok()?;
    Some(help::read(&text))
}

/// The man page folders for `path`, from `manpath` run with it (once per
/// PATH while the worker runs), as `man` itself would search.
fn manpath(path: &str) -> Arc<Vec<PathBuf>> {
    if let Some(dirs) = state().as_ref().and_then(|st| st.manpaths.get(path)) {
        return dirs.clone();
    }
    let out = std::process::Command::new("/usr/bin/manpath")
        .env("PATH", path)
        .env_remove("MANPATH")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let mut dirs: Vec<PathBuf> = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .trim()
            .split(':')
            .filter(|d| d.starts_with('/'))
            .map(PathBuf::from)
            .collect(),
        _ => Vec::new(),
    };
    if dirs.is_empty() {
        // Without `manpath`: each PATH folder's `../share/man`.
        dirs = std::env::split_paths(path)
            .filter_map(|d| d.parent().map(|p| p.join("share/man")))
            .chain([PathBuf::from("/usr/share/man")])
            .filter(|d| d.is_dir())
            .collect();
    }
    let dirs = Arc::new(dirs);
    if let Some(st) = state().as_mut() {
        st.manpaths.insert(path.to_string(), dirs.clone());
    }
    dirs
}

/// `name`'s page in `dirs`: the first in sections 1, 8 and 6, plain or
/// gzipped.
fn find_page(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        return None;
    }
    for dir in dirs {
        for sec in ["1", "8", "6"] {
            for gz in ["", ".gz"] {
                let p = dir.join(format!("man{sec}/{name}.{sec}{gz}"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// A page's text, at most `cap` bytes of it (inflated, for `.gz`).
fn read_page(path: &Path, cap: usize) -> Option<String> {
    use std::io::Read;
    let gz = path.extension().is_some_and(|e| e == "gz");
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(if gz { PAGE_BYTES as u64 } else { cap as u64 })
        .read_to_end(&mut bytes)
        .ok()?;
    if gz {
        bytes = gunzip(&bytes, cap)?;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// A gzip file's contents, at most `cap` bytes of them.
fn gunzip(data: &[u8], cap: usize) -> Option<Vec<u8>> {
    let [0x1f, 0x8b, 8, flags, ..] = *data else { return None };
    let mut at = 10;
    if flags & 4 != 0 {
        let n = u16::from_le_bytes([*data.get(at)?, *data.get(at + 1)?]) as usize;
        at += 2 + n;
    }
    for bit in [8, 16] {
        if flags & bit != 0 {
            at += data.get(at..)?.iter().position(|b| *b == 0)? + 1;
        }
    }
    if flags & 2 != 0 {
        at += 2;
    }
    match miniz_oxide::inflate::decompress_to_vec_with_limit(data.get(at..)?, cap) {
        Ok(v) => Some(v),
        // Cut short at the cap: what was inflated is kept.
        Err(e) if e.status == miniz_oxide::inflate::TINFLStatus::HasMoreOutput => Some(e.output),
        Err(_) => None,
    }
}

/// The subcommands of `command` in the man `sections` folders: pages
/// named `<command>-<name>.<n>`, each with its NAME line.
fn read_subcommands(sections: &[PathBuf], command: &str) -> Subcommands {
    let lead = format!("{command}-");
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for dir in sections {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for e in rd.flatten() {
            let Ok(file) = e.file_name().into_string() else { continue };
            let Some(rest) = file.strip_prefix(&lead) else { continue };
            let rest = rest.strip_suffix(".gz").unwrap_or(rest);
            let Some((name, sec)) = rest.rsplit_once('.') else { continue };
            if name.is_empty() || !matches!(sec, "1" | "8") || found.iter().any(|(n, _)| n == name) {
                continue;
            }
            found.push((name.to_string(), e.path()));
            if found.len() >= SUBCOMMANDS {
                break;
            }
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    let list = found
        .into_iter()
        .map(|(name, path)| {
            let about = read_page(&path, PAGE_HEAD_BYTES).map(|src| man::about(&src)).unwrap_or_default();
            (name, about)
        })
        .collect();
    Subcommands { list }
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

    #[test]
    fn flags_and_subcommands_from_man_pages() {
        let dir = temp("man");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::create_dir_all(dir.join("share/man/man1")).unwrap();
        let page = |name: &str, about: &str, flag: &str| {
            format!(".TH X 1\n.SH NAME\n{name} \\- {about}\n.SH OPTIONS\n.TP\n\\fB{flag}\\fR\nDoes it.\n")
        };
        let man1 = dir.join("share/man/man1");
        std::fs::write(man1.join("decltool.1"), page("decltool", "A tool", "\\-\\-top")).unwrap();
        std::fs::write(man1.join("decltool-run.1"), page("decltool-run", "Run it", "\\-\\-fast")).unwrap();
        // Gzipped, as some packages install them.
        let gz = gzip(page("decltool-stop", "Stop it", "\\-q").as_bytes());
        std::fs::write(man1.join("decltool-stop.1.gz"), gz).unwrap();
        let path = format!("{}:/usr/bin", dir.join("bin").display());
        let woken = Arc::new(AtomicUsize::new(0));
        let m = ready(&woken, |w| manual(&path, "decltool-run", w));
        assert!(m.found);
        assert_eq!(m.page.about, "Run it");
        assert_eq!(m.page.flags[0].name, "--fast");
        let m = ready(&woken, |w| manual(&path, "decltool-stop", w));
        assert_eq!(m.page.flags[0].name, "-q");
        assert!(!ready(&woken, |w| manual(&path, "decltool-none", w)).found);
        let subs = ready(&woken, |w| subcommands(&path, "decltool", w));
        assert_eq!(
            subs.list,
            [
                ("run".to_string(), "Run it".to_string()),
                ("stop".to_string(), "Stop it".to_string())
            ]
        );
    }

    /// A gzip file of `data`.
    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
        out.extend(miniz_oxide::deflate::compress_to_vec(data, 6));
        // CRC and size: not checked here.
        out.extend([0; 8]);
        out
    }

    /// Prints what the real pages on this Mac give, to check by eye:
    /// `cargo test -p terminal-decline real_man_pages -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_man_pages() {
        let path = std::env::var("PATH").unwrap();
        let woken = Arc::new(AtomicUsize::new(0));
        for name in ["ls", "git-fetch", "rg", "cargo-build", "grep", "curl", "ssh", "tar", "less"] {
            let t = Instant::now();
            let m = ready(&woken, |w| manual(&path, name, w));
            println!("== {name}: {} ({} flags, {:?})", m.page.about, m.page.flags.len(), t.elapsed());
            for f in m.page.flags.iter().take(6) {
                println!("  {:<24} {}", f.name, f.about);
            }
        }
        for command in ["git", "cargo"] {
            let t = Instant::now();
            let s = ready(&woken, |w| subcommands(&path, command, w));
            println!("== {command}: {} subcommands ({:?})", s.list.len(), t.elapsed());
            for (n, a) in s.list.iter().take(5) {
                println!("  {n:<20} {a}");
            }
        }
    }

    #[test]
    fn flags_from_help_when_asked() {
        let dir = temp("help");
        *HELP_DIR.lock().unwrap() = Some(dir.join("kept"));
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let exe = dir.join("bin/declhelp");
        std::fs::write(
            &exe,
            "#!/bin/sh\n[ \"$1\" = run ] && { echo '  -f, --fast   Go fast'; exit 0; }\necho 'Commands:'\necho '  run    Run it'\necho '  stop   Stop it'\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:/usr/bin:/bin", dir.join("bin").display());
        let woken = Arc::new(AtomicUsize::new(0));
        // Nothing before it is read.
        assert!(!ready(&woken, |w| manual(&path, "declhelp-run", w)).found);
        let ask = |sub: Option<&str>, name: &str| {
            let (tx, rx) = channel();
            read_help(Help {
                exe: exe.clone(),
                sub: sub.map(str::to_string),
                path: path.clone(),
                name: name.into(),
                done: Box::new(move |r| tx.send(r).unwrap()),
            });
            rx.recv_timeout(Duration::from_secs(10)).unwrap()
        };
        assert_eq!(ask(None, "declhelp"), Ok(2));
        assert_eq!(ask(Some("run"), "declhelp-run"), Ok(2));
        let m = ready(&woken, |w| manual(&path, "declhelp-run", w));
        assert!(m.found);
        assert_eq!(m.page.flags[1].name, "--fast");
        let subs = ready(&woken, |w| subcommands(&path, "declhelp", w));
        assert_eq!(subs.list[0], ("run".to_string(), "Run it".to_string()));
        // A program that hangs is stopped.
        std::fs::write(&exe, "#!/bin/sh\nsleep 30\n").unwrap();
        let t = Instant::now();
        assert!(ask(None, "declhelp-slow").is_err());
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn escapes_and_overstrikes_taken_out() {
        assert_eq!(clean("\x1b[1mbold\x1b[0m _\x08u\r\n"), "bold u\n");
    }

    /// Prints what real programs' `--help` gives:
    /// `cargo test -p terminal-decline real_help -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_help() {
        let path = std::env::var("PATH").unwrap();
        for (exe, sub) in [
            ("gh", None),
            ("gh", Some("pr")),
            ("kubectl", Some("get")),
            ("npm", None),
            ("cargo", None),
            ("brew", None),
            ("node", None),
        ] {
            let Some(exe_path) = std::env::split_paths(&path).map(|d| d.join(exe)).find(|p| p.is_file()) else {
                continue;
            };
            let t = Instant::now();
            let job = Help {
                exe: exe_path,
                sub: sub.map(str::to_string),
                path: path.clone(),
                name: exe.into(),
                done: Box::new(|_| {}),
            };
            let h = help::read(&help_output(&job).unwrap_or_default());
            println!(
                "== {exe} {sub:?}: {} flags, {} subcommands ({:?})",
                h.page.flags.len(),
                h.subcommands.len(),
                t.elapsed()
            );
            for f in h.page.flags.iter().take(4) {
                println!("  {:<22} {}", f.name, f.about);
            }
            for (n, a) in h.subcommands.iter().take(4) {
                println!("  {n:<22} {a}");
            }
        }
    }

    #[test]
    fn commands_counted_from_history() {
        let dir = temp("history");
        let file = dir.join(".zsh_history");
        std::fs::write(
            &file,
            ": 1700000000:0;git status\n: 1700000001:0;git push && cargo test\nls -la\nfor f in x\\\ndo echo $f\\\ndone\n",
        )
        .unwrap();
        let woken = Arc::new(AtomicUsize::new(0));
        let h = ready(&woken, |w| history(&file, w));
        assert_eq!(
            (h.count("git"), h.count("cargo"), h.count("ls"), h.count("echo"), h.count("do")),
            (2, 1, 1, 1, 0)
        );
        // Added to: only the new lines are read.
        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        std::io::Write::write_all(&mut f, b": 1700000002:0;git log\n").unwrap();
        drop(f);
        let next = |w: &dyn Fn() -> Wake| history(&file, w).filter(|h| h.count("git") == 3);
        let deadline = Instant::now() + Duration::from_secs(10);
        while next(&|| Box::new(|| {})).is_none() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(unmetafy(&[b'a', 0x83, 0xa9 ^ 32]), String::from_utf8_lossy(&[b'a', 0xa9]));
    }

    /// Times reading the real history file:
    /// `cargo test -p terminal-decline real_history -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_history() {
        let file = Path::new(&std::env::var("HOME").unwrap()).join(".zsh_history");
        let t = Instant::now();
        let h = read_history(&file, None);
        let mut top: Vec<_> = h.counts.iter().collect();
        top.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
        println!(
            "{} commands, {} bytes in {:?}; top: {:?}",
            h.counts.len(),
            h.read_to,
            t.elapsed(),
            top[..top.len().min(5)].iter().map(|(n, _)| n).collect::<Vec<_>>()
        );
    }
}
