//! Dogfood builds restart into a newer build as soon as one is installed.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

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
            },
            SavedWindow {
                cwd: None,
                frame: [5.0, 6.0, 300.0, 200.0],
                tab_group: Some(1),
                selected: true,
                panes: None,
                command: None,
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
