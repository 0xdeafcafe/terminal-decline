//! Dogfood builds restart into a newer build once one is installed.
//!
//! `scripts/dogfood.sh` marks the bundle with `Contents/Resources/dogfood`.
//! Such a build watches its own executable with kqueue (a thread blocked in
//! the kernel: no timers, no wakeups while nothing changes). When the file is
//! replaced it saves each window's directory and frame, starts the new build
//! with `--restore <file>`, and quits. Tabs are saved in order with the
//! index of their tab group, and reopen as tabs of one window. A window
//! with several panes also saves their layout ([`SavedPanes`]). A tab (or
//! pane) whose shell was running a program saves its command line, which
//! the new build types at the shell's prompt without running it.
//!
//! The restart never ends a program the user is running. When the new
//! build arrives while any pane runs something other than its shell at
//! the prompt, the old build carries on (its library is already loaded)
//! and says once that it was updated, with a Restart button and Terminal
//! Decline › Restart to Update. It restarts by itself once every pane is
//! at its prompt and nobody has used the app for [`RESTART_IDLE`]
//! ([`busy`], [`when_to_restart`]), else at the next launch.

use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SavedWindow {
    pub cwd: Option<String>,
    /// x, y, width, height in screen points.
    pub frame: [f64; 4],
    /// Windows with the same index were tabs of one window, saved in tab
    /// order. `None` (and files from builds before tabs): a window of its
    /// own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_group: Option<usize>,
    /// The tab that was in front in its group.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub selected: bool,
    /// Its panes, when it has more than one (`cwd` is then the focused
    /// pane's). `None` (and files from builds before splits): one pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panes: Option<SavedPanes>,
    /// The program that was running in its shell's foreground, as a
    /// command line quoted for that shell (`nvim 'my file.rs'`), typed at
    /// the new shell's prompt but not run (see
    /// [`crate::running::foreground_command`]). `None` at the prompt, for
    /// a window started with `-e PROGRAM` or a pin's (they run their
    /// program again anyway), for the Settings tab, and in files from
    /// builds before this. With panes, the focused pane's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Its history, saved for the restart: a file of
    /// [`crate::saved_history`]'s, shown again above the new shell. `None`
    /// with panes (each pane has its own), on the alternate screen, with
    /// `restore_scrollback = false`, and in files from builds before this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<String>,
}

/// A window's panes as saved: the tree with its shares, each pane's
/// working directory, and which pane had focus and which was zoomed (by
/// their place in tree order).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SavedPanes {
    pub root: SavedNode,
    pub focused: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoomed: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SavedNode {
    /// Written as `{"pane": {"cwd": …}}`, as before panes saved commands.
    Pane(SavedPane),
    Split {
        dir: crate::panes::Dir,
        /// `a`'s share.
        ratio: f32,
        a: Box<SavedNode>,
        b: Box<SavedNode>,
    },
}

/// One pane as saved: its working directory, the command it was
/// running (as [`SavedWindow::command`]), and whether it ran the window's
/// own program.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SavedPane {
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// It ran the program the window was opened with (`-e PROGRAM`, a
    /// pin's, `command` in the config), which it runs again; else (a
    /// split) the user's shell.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub program: bool,
    /// Its history's file, as [`SavedWindow::history`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<String>,
}

/// Saved windows as they reopen: one list per window, its tabs in order.
/// Tabs of one group stay together where the group was first seen, even if
/// the file lists them apart.
pub fn tab_groups(saved: Vec<SavedWindow>) -> Vec<Vec<SavedWindow>> {
    let mut out: Vec<Vec<SavedWindow>> = Vec::new();
    let mut index_of: Vec<(usize, usize)> = Vec::new();
    for w in saved {
        match w.tab_group {
            Some(g) => match index_of.iter().find(|(group, _)| *group == g) {
                Some(&(_, i)) => out[i].push(w),
                None => {
                    index_of.push((g, out.len()));
                    out.push(vec![w]);
                }
            },
            None => out.push(vec![w]),
        }
    }
    out
}

/// The bundle this process's executable is in, if any: the app (its
/// launcher, or the static binary in a bundle from before the launcher), or
/// a pin (its stub or clone), which has no dogfood marker.
pub fn own_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let contents = exe.parent()?.parent()?;
    (contents.file_name()? == "Contents").then(|| contents.parent().map(Path::to_path_buf))?
}

/// The file whose replacement means a new build is installed: the library
/// the launcher loaded, in a bundle that has one, else the executable.
pub fn running_code(bundle: &Path, exe: PathBuf) -> PathBuf {
    let library = bundle.join("Contents/Frameworks/libterminal_decline.dylib");
    if library.is_file() { library } else { exe }
}

pub fn is_dogfood_bundle(bundle: &Path) -> bool {
    bundle.join("Contents/Resources/dogfood").exists()
}

/// The bundle's name after the rename to Terminal Decline (E.17).
pub const RENAMED_BUNDLE: &str = "Terminal Decline.app";

/// The bundle to restart into once `bundle`'s executable changed: `bundle`
/// itself when a new build replaced it, else (the dogfood script removed
/// the build from before the rename) `Terminal Decline.app` in the same
/// folder, if it is there.
pub fn relaunch_target(bundle: &Path) -> Option<PathBuf> {
    let is_app = |b: &Path| b.join("Contents/Info.plist").is_file();
    if is_app(bundle) {
        return Some(bundle.to_path_buf());
    }
    let renamed = bundle.parent()?.join(RENAMED_BUNDLE);
    (renamed != bundle && is_app(&renamed)).then_some(renamed)
}

/// Calls `on_change` (on a background thread) once the executable at `exe`
/// is replaced, deleted or rewritten.
pub fn watch(exe: PathBuf, on_change: impl FnOnce() + Send + 'static) -> std::io::Result<()> {
    let path = std::ffi::CString::new(exe.as_os_str().as_encoded_bytes()).map_err(std::io::Error::other)?;
    let fd: RawFd = unsafe { libc::open(path.as_ptr(), libc::O_EVTONLY) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let kq = unsafe { libc::kqueue() };
    if kq < 0 {
        unsafe { libc::close(fd) };
        return Err(std::io::Error::last_os_error());
    }
    // Register before returning, so a change right after `watch` is seen.
    let change = libc::kevent {
        ident: fd as usize,
        filter: libc::EVFILT_VNODE,
        flags: libc::EV_ADD | libc::EV_CLEAR,
        fflags: libc::NOTE_DELETE | libc::NOTE_WRITE | libc::NOTE_RENAME | libc::NOTE_ATTRIB | libc::NOTE_REVOKE,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    if unsafe { libc::kevent(kq, &change, 1, std::ptr::null_mut(), 0, std::ptr::null()) } < 0 {
        let e = std::io::Error::last_os_error();
        unsafe {
            libc::close(fd);
            libc::close(kq);
        }
        return Err(e);
    }
    let before = identity(&exe);
    std::thread::Builder::new().name("build-watch".into()).spawn(move || {
        let mut event: libc::kevent = unsafe { std::mem::zeroed() };
        loop {
            let n = unsafe { libc::kevent(kq, std::ptr::null(), 0, &mut event, 1, std::ptr::null()) };
            if n < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                return;
            }
            if n <= 0 {
                continue;
            }
            // Let the installer finish putting the new bundle in place.
            std::thread::sleep(Duration::from_millis(400));
            // macOS touches the executable's attributes when it runs it
            // again (the Settings tab runs this same binary) or checks its
            // signature: only a different file is a new build.
            if before.is_some() && identity(&exe) == before {
                continue;
            }
            break;
        }
        unsafe {
            libc::close(fd);
            libc::close(kq);
        }
        on_change();
    })?;
    Ok(())
}

/// What tells one build's executable from another: the file, its size
/// and when it was last written. `None` once it is gone.
fn identity(exe: &Path) -> Option<(u64, u64, u64, i64, i64)> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(exe).ok()?;
    Some((m.dev(), m.ino(), m.size(), m.mtime(), m.mtime_nsec()))
}

fn restore_file() -> PathBuf {
    let base = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join(format!("Library/Caches/terminal-decline/restore-{}.json", std::process::id()))
}

pub fn save(windows: &[SavedWindow]) -> std::io::Result<PathBuf> {
    let path = restore_file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, serde_json::to_vec(windows).map_err(std::io::Error::other)?)?;
    Ok(path)
}

/// Reads and removes a restore file.
pub fn take(path: &Path) -> Vec<SavedWindow> {
    let list = std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let _ = std::fs::remove_file(path);
    list
}

/// Starts a fresh instance of `bundle` that reopens `restore`.
pub fn relaunch(bundle: &Path, restore: &Path) -> std::io::Result<()> {
    let status = std::process::Command::new("/usr/bin/open")
        .arg("-n")
        .arg(bundle)
        .arg("--args")
        .arg("--restore")
        .arg(restore)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("open exited with {status}")))
    }
}

/// How long nobody must have used the app (no key or mouse input to it)
/// before a new build waiting to take over restarts it by itself.
pub const RESTART_IDLE: Duration = Duration::from_secs(120);

/// With the test control on, the seconds [`RESTART_IDLE`] is cut to, for
/// the smoke tests.
pub const TEST_IDLE_ENV: &str = "DECLINE_TEST_RESTART_IDLE";

/// Written by `scripts/dogfood.sh --now` just before it installs, under
/// `$HOME`: the running build restarts into the new one at once, whatever
/// runs ([`take_restart_now`]).
pub const RESTART_NOW: &str = "Library/Caches/terminal-decline/restart-now";

/// Whether the install just seen asked to restart at once
/// ([`RESTART_NOW`] in `home`, written in the last minute); the file is
/// removed either way, so only this install's counts.
pub fn take_restart_now(home: &Path) -> bool {
    let file = home.join(RESTART_NOW);
    let fresh = std::fs::metadata(&file)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().is_ok_and(|e| e < Duration::from_secs(60)));
    let _ = std::fs::remove_file(&file);
    fresh
}

/// One pane as the restart sees it, read when deciding (nothing polls).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaneNow {
    /// What the shell's OSC 133 marks say: `None` without marks (no
    /// shell integration), `Some(None)` at its prompt, `Some(Some(c))`
    /// running the command typed as `c` (empty if the marks didn't show
    /// it).
    pub marks: Option<Option<String>>,
    /// The program in its terminal's foreground, when that isn't its
    /// shell (the PTY's foreground process group is another than the
    /// shell's pid): [`crate::running::foreground_program`].
    pub foreground: Option<String>,
    /// It runs the window's own program (`-e PROGRAM`, `command` in the
    /// config) rather than the user's shell, named here: a restart would
    /// end it too.
    pub program: Option<String>,
}

/// What `pane` runs that a restart would end, named for the question
/// asked before one, or `None` when it is at its shell's prompt. Busy if
/// the marks say a command runs or the foreground isn't the shell: the
/// marks alone miss a program started without them (no shell
/// integration), the foreground alone a shell builtin or function.
pub fn busy(pane: &PaneNow) -> Option<String> {
    if let Some(p) = &pane.program {
        return Some(p.clone());
    }
    match (&pane.marks, &pane.foreground) {
        (_, Some(f)) => Some(f.clone()),
        (Some(Some(c)), None) => Some(
            c.split_whitespace()
                .next()
                .map(|w| w.rsplit('/').next().unwrap_or(w).to_string())
                .unwrap_or_else(|| "a command".into()),
        ),
        _ => None,
    }
}

/// How often a new build waiting for programs to end looks again by
/// itself (a command finishing in a pane with marks, or a window
/// closing, looks sooner).
pub const BUSY_LOOK_AGAIN: Duration = Duration::from_secs(120);

/// When a new build waiting to take over restarts the app: `None` for
/// now (no pane runs anything and nobody has used the app for `wait`),
/// else look again after the time given: with a program running,
/// [`BUSY_LOOK_AGAIN`]; else the rest of `wait` since the last input.
pub fn when_to_restart(busy: &[String], idle: Duration, wait: Duration) -> Option<Duration> {
    if !busy.is_empty() {
        return Some(BUSY_LOOK_AGAIN);
    }
    (idle < wait).then(|| wait - idle)
}

/// The command to type at a restored pane's prompt, from what was known
/// as the restart saved it:
///
/// 1. `line`, the command line the shell sent with its OSC 133 mark
///    (zsh and fish do): as typed, so an alias or a function stays one
///    and `FOO=1 cmd` keeps its `FOO=1`, which the process's arguments
///    lose;
/// 2. else `foreground`, the foreground process's command line
///    ([`crate::running::foreground_command`]);
/// 3. else `shown`, the text the marks showed on the screen (bash, or a
///    program whose arguments can't be read): only its first line,
///    read back from the screen, so not when it fills
///    [`SHOWN_MAX`] and may have been cut.
///
/// Text with control characters (the line feed of a multi-line command)
/// isn't typed: it could run before Enter.
pub fn command_to_type(line: Option<&str>, foreground: Option<String>, shown: Option<&str>) -> Option<String> {
    let usable = |c: &&str| !c.is_empty() && c.len() <= crate::running::MAX_COMMAND && !c.chars().any(char::is_control);
    let line = line.map(str::trim).filter(usable);
    let shown = shown.map(str::trim).filter(usable).filter(|c| c.chars().count() < SHOWN_MAX);
    line.map(str::to_string).or(foreground).or(shown.map(str::to_string))
}

/// The most characters of a command's first line the OSC 133 marks show
/// (term-core's `COMMAND_TEXT_MAX`).
pub const SHOWN_MAX: usize = 200;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn the_command_typed_is_the_shells_line_first() {
        let fg = || Some("/opt/homebrew/bin/rush".to_string());
        assert_eq!(command_to_type(Some("rush"), fg(), Some("rush")).as_deref(), Some("rush"));
        assert_eq!(command_to_type(Some(" FOO=1 rush "), fg(), None).as_deref(), Some("FOO=1 rush"));
        // No line from the shell (bash): the process's command line, then
        // what the screen showed.
        assert_eq!(command_to_type(None, fg(), Some("rush")), fg());
        assert_eq!(command_to_type(None, None, Some("sudo rush")).as_deref(), Some("sudo rush"));
        // Shown text that may have been cut isn't typed.
        let long = "x".repeat(SHOWN_MAX);
        assert_eq!(command_to_type(None, None, Some(&long)), None);
        // A line that could run before Enter isn't typed.
        assert_eq!(command_to_type(Some("echo a\nrm x"), fg(), None), fg());
        assert_eq!(command_to_type(Some("a\x1bb"), None, None), None);
        assert_eq!(command_to_type(Some("   "), None, None), None);
        // Backslashes are only text.
        assert_eq!(command_to_type(Some(r"echo a\nb"), None, None).as_deref(), Some(r"echo a\nb"));
        assert_eq!(command_to_type(None, None, None), None);
    }

    fn pane(marks: Option<Option<&str>>, foreground: Option<&str>) -> PaneNow {
        PaneNow {
            marks: marks.map(|m| m.map(str::to_string)),
            foreground: foreground.map(str::to_string),
            program: None,
        }
    }

    #[test]
    fn busy_from_the_marks_and_the_foreground() {
        // At the prompt, with marks or without.
        assert_eq!(busy(&pane(Some(None), None)), None);
        assert_eq!(busy(&pane(None, None)), None);
        // The marks say a command runs: named by the foreground program,
        // else (a function, a builtin) by the first word typed.
        assert_eq!(busy(&pane(Some(Some("rush --resume")), Some("rush"))).as_deref(), Some("rush"));
        assert_eq!(busy(&pane(Some(Some("~/bin/rush x")), None)).as_deref(), Some("rush"));
        assert_eq!(busy(&pane(Some(Some("")), None)).as_deref(), Some("a command"));
        // No marks (no shell integration): the foreground process group.
        assert_eq!(busy(&pane(None, Some("vim"))).as_deref(), Some("vim"));
        // Marks at the prompt, yet another program has the terminal.
        assert_eq!(busy(&pane(Some(None), Some("ssh"))).as_deref(), Some("ssh"));
        // A window's own program always counts.
        let p = PaneNow {
            program: Some("htop".into()),
            ..PaneNow::default()
        };
        assert_eq!(busy(&p).as_deref(), Some("htop"));
    }

    #[test]
    fn restart_now_counts_once_and_only_fresh() {
        let home = std::env::temp_dir().join(format!("terminal-decline-restart-now-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        assert!(!take_restart_now(&home), "no file");
        let file = home.join(RESTART_NOW);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"").unwrap();
        assert!(take_restart_now(&home));
        assert!(!file.exists() && !take_restart_now(&home), "only once");
        // Left by an install long ago that no running build saw.
        std::fs::write(&file, b"").unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(old).unwrap();
        assert!(!take_restart_now(&home));
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn restarts_only_at_the_prompt_once_idle() {
        let wait = RESTART_IDLE;
        let secs = Duration::from_secs;
        // Every pane at its prompt, nobody about: now.
        assert_eq!(when_to_restart(&[], wait, wait), None);
        assert_eq!(when_to_restart(&[], wait * 3, wait), None);
        // At the prompt, but used 30 s ago: the rest of the wait.
        assert_eq!(when_to_restart(&[], secs(30), wait), Some(wait - secs(30)));
        // A program runs: not now, however idle; look again later.
        let busy = ["rush".to_string()];
        assert_eq!(when_to_restart(&busy, wait * 10, wait), Some(BUSY_LOOK_AGAIN));
        assert_eq!(when_to_restart(&busy, Duration::ZERO, wait), Some(BUSY_LOOK_AGAIN));
        // An update asked for (no idle wait): now once nothing runs, and
        // while something does, not in a loop.
        assert_eq!(when_to_restart(&[], Duration::ZERO, Duration::ZERO), None);
        assert_eq!(when_to_restart(&busy, Duration::ZERO, Duration::ZERO), Some(BUSY_LOOK_AGAIN));
    }

    #[test]
    fn restore_round_trip() {
        let dir = std::env::temp_dir().join(format!("terminal-decline-reload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.json");
        let w = vec![
            SavedWindow {
                cwd: Some("/tmp".into()),
                frame: [1.0, 2.0, 300.0, 200.0],
                tab_group: None,
                selected: false,
                panes: None,
                command: None,
                history: None,
            },
            SavedWindow {
                cwd: None,
                frame: [5.0, 6.0, 300.0, 200.0],
                tab_group: Some(1),
                selected: true,
                panes: None,
                command: None,
                history: None,
            },
            SavedWindow {
                cwd: Some("/b".into()),
                frame: [5.0, 6.0, 300.0, 200.0],
                tab_group: None,
                selected: false,
                panes: Some(SavedPanes {
                    root: SavedNode::Split {
                        dir: crate::panes::Dir::Right,
                        ratio: 0.25,
                        a: Box::new(SavedNode::Pane(SavedPane {
                            cwd: Some("/a".into()),
                            command: None,
                            ..SavedPane::default()
                        })),
                        b: Box::new(SavedNode::Split {
                            dir: crate::panes::Dir::Down,
                            ratio: 0.5,
                            a: Box::new(SavedNode::Pane(SavedPane {
                                cwd: Some("/b".into()),
                                command: None,
                                ..SavedPane::default()
                            })),
                            b: Box::new(SavedNode::Pane(SavedPane::default())),
                        }),
                    },
                    focused: 1,
                    zoomed: None,
                }),
                command: None,
                history: None,
            },
        ];
        std::fs::write(&path, serde_json::to_vec(&w).unwrap()).unwrap();
        assert_eq!(take(&path), w);
        assert!(!path.exists());
        assert!(take(&path).is_empty());
    }

    fn window(cwd: &str, tab_group: Option<usize>, selected: bool) -> SavedWindow {
        SavedWindow {
            cwd: Some(cwd.into()),
            frame: [0.0, 0.0, 300.0, 200.0],
            tab_group,
            selected,
            panes: None,
            command: None,
            history: None,
        }
    }

    /// The running build writes the file and the new one reads it, so a
    /// file from a build before tabs must still restore, as plain windows.
    #[test]
    fn files_from_before_tabs_still_restore() {
        let old = br#"[{"cwd":"/a","frame":[1.0,2.0,3.0,4.0]},{"cwd":null,"frame":[5.0,6.0,7.0,8.0]}]"#;
        let list: Vec<SavedWindow> = serde_json::from_slice(old).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|w| w.tab_group.is_none() && !w.selected && w.panes.is_none()));
        assert_eq!(tab_groups(list).iter().map(Vec::len).collect::<Vec<_>>(), [1, 1]);
        // Plain windows are written as before.
        let json = serde_json::to_string(&window("/a", None, false)).unwrap();
        assert_eq!(json, r#"{"cwd":"/a","frame":[0.0,0.0,300.0,200.0]}"#);
    }

    /// Commands saved for tabs and panes come back as they were, and a
    /// file from a build before commands were saved still restores, with
    /// none: the field is left out when there is nothing to save, so such
    /// a file is what this build writes for tabs at their prompt.
    #[test]
    fn commands_round_trip_and_old_files_have_none() {
        let old = br#"[{"cwd":"/a","frame":[1.0,2.0,3.0,4.0],"tab_group":0,"selected":true,
            "panes":{"root":{"split":{"dir":"right","ratio":0.5,"a":{"pane":{"cwd":"/a"}},"b":{"pane":{"cwd":null}}}},"focused":0}},
            {"cwd":null,"frame":[5.0,6.0,7.0,8.0]}]"#;
        let list: Vec<SavedWindow> = serde_json::from_slice(old).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|w| w.command.is_none()));
        let Some(SavedNode::Split { a, b, .. }) = list[0].panes.as_ref().map(|p| &p.root) else {
            panic!("{list:?}")
        };
        assert_eq!(
            **a,
            SavedNode::Pane(SavedPane {
                cwd: Some("/a".into()),
                command: None,
                ..SavedPane::default()
            })
        );
        assert_eq!(**b, SavedNode::Pane(SavedPane::default()));
        // Written back the same way.
        let json = serde_json::to_string(&list[0].panes).unwrap();
        assert!(json.contains(r#""a":{"pane":{"cwd":"/a"}},"b":{"pane":{"cwd":null}}"#), "{json}");

        let mut w = window("/a", Some(0), true);
        w.command = Some("nvim 'my file.rs'".into());
        w.panes = Some(SavedPanes {
            root: SavedNode::Split {
                dir: crate::panes::Dir::Down,
                ratio: 0.5,
                a: Box::new(SavedNode::Pane(SavedPane {
                    cwd: Some("/a".into()),
                    command: Some("ssh host".into()),
                    ..SavedPane::default()
                })),
                b: Box::new(SavedNode::Pane(SavedPane::default())),
            },
            focused: 0,
            zoomed: None,
        });
        let json = serde_json::to_string(&[w.clone(), window("/b", None, false)]).unwrap();
        assert!(json.contains(r#""command":"nvim 'my file.rs'""#), "{json}");
        assert!(json.contains(r#"{"pane":{"cwd":"/a","command":"ssh host"}}"#), "{json}");
        let back: Vec<SavedWindow> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, [w, window("/b", None, false)]);
    }

    /// Each window's and pane's history file comes back as it was; files
    /// from builds before this name none, and none is written for a
    /// window without one.
    #[test]
    fn history_files_round_trip_and_old_files_have_none() {
        let old = br#"[{"cwd":"/a","frame":[1.0,2.0,3.0,4.0],"panes":{"root":{"pane":{"cwd":"/a"}},"focused":0}}]"#;
        let list: Vec<SavedWindow> = serde_json::from_slice(old).unwrap();
        assert!(list[0].history.is_none());
        assert!(matches!(&list[0].panes.as_ref().unwrap().root, SavedNode::Pane(p) if p.history.is_none()));
        assert!(!serde_json::to_string(&list).unwrap().contains("history"));
        let mut w = window("/a", None, false);
        w.history = Some("1-2-0.hist".into());
        w.panes = Some(SavedPanes {
            root: SavedNode::Pane(SavedPane {
                history: Some("1-2-1.hist".into()),
                ..SavedPane::default()
            }),
            focused: 0,
            zoomed: None,
        });
        let json = serde_json::to_string(&[w.clone()]).unwrap();
        assert!(
            json.contains(r#""history":"1-2-0.hist""#) && json.contains(r#""history":"1-2-1.hist""#),
            "{json}"
        );
        let back: Vec<SavedWindow> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, [w]);
    }

    #[test]
    fn tabs_reopen_together_in_order() {
        let saved = vec![
            window("/one", None, false),
            window("/a1", Some(0), false),
            window("/a2", Some(0), true),
            window("/b1", Some(1), true),
            window("/two", None, false),
            // Out of place in the file: still a tab of group 0, at the end.
            window("/a3", Some(0), false),
        ];
        let json = serde_json::to_vec(&saved).unwrap();
        let back: Vec<SavedWindow> = serde_json::from_slice(&json).unwrap();
        assert_eq!(back, saved);
        let groups = tab_groups(back);
        let groups: Vec<Vec<&str>> = groups
            .iter()
            .map(|g| g.iter().map(|w| w.cwd.as_deref().unwrap()).collect())
            .collect();
        assert_eq!(groups, [vec!["/one"], vec!["/a1", "/a2", "/a3"], vec!["/b1"], vec!["/two"]]);
        assert!(tab_groups(Vec::new()).is_empty());
    }

    #[test]
    fn restarts_into_the_renamed_bundle_once_this_one_is_gone() {
        let dir = std::env::temp_dir().join(format!("terminal-decline-relaunch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let app = |name: &str| {
            let b = dir.join(name);
            std::fs::create_dir_all(b.join("Contents")).unwrap();
            std::fs::write(b.join("Contents/Info.plist"), b"").unwrap();
            b
        };
        let old = app("Decline.app");
        // Replaced in place: restart into the same bundle.
        assert_eq!(relaunch_target(&old), Some(old.clone()));
        // Removed, with nothing beside it: nowhere to go.
        std::fs::remove_dir_all(&old).unwrap();
        assert_eq!(relaunch_target(&old), None);
        // Removed, with the renamed build beside it.
        let renamed = app(RENAMED_BUNDLE);
        assert_eq!(relaunch_target(&old), Some(renamed.clone()));
        assert_eq!(relaunch_target(&renamed), Some(renamed.clone()));
        std::fs::remove_dir_all(&renamed).unwrap();
        assert_eq!(relaunch_target(&renamed), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watch_fires_when_the_file_is_replaced() {
        let dir = std::env::temp_dir().join(format!("terminal-decline-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("terminal-decline");
        std::fs::write(&exe, b"old").unwrap();
        let (tx, rx) = mpsc::channel();
        watch(exe.clone(), move || tx.send(()).unwrap()).unwrap();
        // How dogfood.sh installs: the old file goes away, a new one appears.
        std::fs::remove_file(&exe).unwrap();
        std::fs::write(&exe, b"new").unwrap();
        rx.recv_timeout(Duration::from_secs(5)).expect("no change seen");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watch_ignores_attribute_changes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("terminal-decline-watch-attr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("terminal-decline");
        std::fs::write(&exe, b"build").unwrap();
        let (tx, rx) = mpsc::channel();
        watch(exe.clone(), move || tx.send(()).unwrap()).unwrap();
        // What running the same executable again, or a signature check,
        // does to it: its attributes change, the file doesn't.
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            rx.recv_timeout(Duration::from_secs(1)).is_err(),
            "restarted for an attribute change"
        );
        // A real new build is still seen afterwards.
        std::fs::remove_file(&exe).unwrap();
        std::fs::write(&exe, b"new build").unwrap();
        rx.recv_timeout(Duration::from_secs(5)).expect("no change seen");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
