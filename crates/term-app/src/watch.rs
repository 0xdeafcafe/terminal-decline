//! Watching the config files for changes, without polling.
//!
//! One thread blocks in `kevent` on a kqueue holding a vnode watch on each
//! file and on the directory it is in (editors save by writing a new file
//! and renaming it over the old one, which only the directory sees). A file
//! that doesn't exist yet is watched through the nearest directory that
//! does, so creating it is seen too. A symlinked file is also watched at its
//! target. Nothing runs until the kernel reports a change: no timers, no
//! wakeups while idle.
//!
//! After an event the thread waits a moment for the editor to finish,
//! arms the watches again (the files may be new ones) and compares the
//! files' contents with what it last read. Only a real change calls
//! `on_change`, so touching a file or saving it unchanged reloads nothing.

use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long after an event the files are read: editors write in steps
/// (a temp file, a rename, attributes).
const SETTLE: Duration = Duration::from_millis(40);

/// The user event that wakes the thread when the list of files changes.
const WAKE: usize = 1;

pub struct Watcher {
    kq: RawFd,
    paths: Arc<Mutex<Vec<PathBuf>>>,
}

impl Watcher {
    /// Watches `paths`, calling `on_change` (on the watcher's thread) each
    /// time one of them is created, deleted or changes contents.
    pub fn start(paths: Vec<PathBuf>, on_change: impl Fn() + Send + 'static) -> std::io::Result<Watcher> {
        let kq = unsafe { libc::kqueue() };
        if kq < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let wake = kevent(WAKE, libc::EVFILT_USER, libc::EV_ADD | libc::EV_CLEAR, 0);
        if unsafe { libc::kevent(kq, &wake, 1, std::ptr::null_mut(), 0, std::ptr::null()) } < 0 {
            let e = std::io::Error::last_os_error();
            unsafe { libc::close(kq) };
            return Err(e);
        }
        let shared = Arc::new(Mutex::new(paths));
        let mine = shared.clone();
        // Armed and read before returning, so a change right after `start`
        // is seen.
        let first = mine.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut fds = arm(kq, &first);
        let mut seen = snapshot(&first);
        std::thread::Builder::new().name("config-watch".into()).spawn(move || {
            let mut events: [libc::kevent; 16] = unsafe { std::mem::zeroed() };
            loop {
                let n = unsafe { libc::kevent(kq, std::ptr::null(), 0, events.as_mut_ptr(), events.len() as i32, std::ptr::null()) };
                if n < 0 {
                    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return;
                }
                let only_wake = events[..n as usize].iter().all(|e| e.filter == libc::EVFILT_USER);
                if !only_wake {
                    std::thread::sleep(SETTLE);
                }
                for fd in fds.drain(..) {
                    unsafe { libc::close(fd) };
                }
                let current = mine.lock().unwrap_or_else(|e| e.into_inner()).clone();
                fds = arm(kq, &current);
                let now = snapshot(&current);
                // A new list of files (after a reload) is read again without
                // reporting it: the reload already read them.
                if now != seen && !only_wake {
                    seen = now;
                    on_change();
                } else {
                    seen = now;
                }
            }
        })?;
        Ok(Watcher { kq, paths: shared })
    }

    /// Watches `paths` instead, from now on.
    pub fn set_paths(&self, paths: Vec<PathBuf>) {
        let mut current = self.paths.lock().unwrap_or_else(|e| e.into_inner());
        if *current == paths {
            return;
        }
        *current = paths;
        drop(current);
        let wake = kevent(WAKE, libc::EVFILT_USER, 0, libc::NOTE_TRIGGER);
        unsafe { libc::kevent(self.kq, &wake, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
    }
}

fn kevent(ident: usize, filter: i16, flags: u16, fflags: u32) -> libc::kevent {
    libc::kevent {
        ident,
        filter,
        flags,
        fflags,
        data: 0,
        udata: std::ptr::null_mut(),
    }
}

/// Each file's contents, `None` for one that can't be read.
fn snapshot(paths: &[PathBuf]) -> Vec<Option<Vec<u8>>> {
    paths.iter().map(|p| std::fs::read(p).ok()).collect()
}

/// What to watch for `paths`: each file that exists, the directory it is in
/// (or the nearest one that exists), and the same for a symlink's target.
pub fn watch_list(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut add = |p: PathBuf| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    for path in paths {
        let mut targets = vec![path.clone()];
        if let Ok(real) = path.canonicalize()
            && real != *path
        {
            targets.push(real);
        }
        for p in targets {
            if p.exists() {
                add(p.clone());
            }
            let mut dir = p.parent();
            while let Some(d) = dir {
                if d.is_dir() {
                    add(d.to_path_buf());
                    break;
                }
                dir = d.parent();
            }
        }
    }
    out
}

/// Registers a vnode watch for each of `paths`' watch list; returns the
/// descriptors, to close before arming again.
fn arm(kq: RawFd, paths: &[PathBuf]) -> Vec<RawFd> {
    let mut fds = Vec::new();
    for p in watch_list(paths) {
        let Some(fd) = open_for_events(&p) else { continue };
        let change = kevent(
            fd as usize,
            libc::EVFILT_VNODE,
            libc::EV_ADD | libc::EV_CLEAR,
            libc::NOTE_DELETE | libc::NOTE_WRITE | libc::NOTE_EXTEND | libc::NOTE_RENAME | libc::NOTE_ATTRIB | libc::NOTE_REVOKE,
        );
        if unsafe { libc::kevent(kq, &change, 1, std::ptr::null_mut(), 0, std::ptr::null()) } < 0 {
            unsafe { libc::close(fd) };
            continue;
        }
        fds.push(fd);
    }
    fds
}

fn open_for_events(path: &Path) -> Option<RawFd> {
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_EVTONLY) };
    (fd >= 0).then_some(fd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::sync::mpsc;

    const WAIT: Duration = Duration::from_secs(5);

    fn watcher(paths: Vec<PathBuf>) -> (Watcher, mpsc::Receiver<()>) {
        let (tx, rx) = mpsc::channel();
        let w = Watcher::start(paths, move || {
            let _ = tx.send(());
        })
        .unwrap();
        (w, rx)
    }

    /// No more reports come, within a short wait.
    fn quiet(rx: &mpsc::Receiver<()>) -> bool {
        rx.recv_timeout(Duration::from_millis(300)).is_err()
    }

    #[test]
    fn sees_edits_in_place_and_by_rename() {
        let dir = TempDir::new("watch-edit");
        let file = dir.join("config.toml");
        std::fs::write(&file, "a = 1\n").unwrap();
        let (_w, rx) = watcher(vec![file.clone()]);
        // In place.
        std::fs::write(&file, "a = 2\n").unwrap();
        rx.recv_timeout(WAIT).expect("in-place write not seen");
        assert!(quiet(&rx));
        // As editors save: a new file renamed over the old one.
        let tmp = dir.join(".config.toml.swp");
        std::fs::write(&tmp, "a = 3\n").unwrap();
        std::fs::rename(&tmp, &file).unwrap();
        rx.recv_timeout(WAIT).expect("rename not seen");
        assert!(quiet(&rx));
        // Then in place again: the new file is watched.
        std::fs::write(&file, "a = 4\n").unwrap();
        rx.recv_timeout(WAIT).expect("write to the renamed file not seen");
    }

    #[test]
    fn same_contents_or_other_files_report_nothing() {
        let dir = TempDir::new("watch-same");
        let file = dir.join("config.toml");
        std::fs::write(&file, "a = 1\n").unwrap();
        let (_w, rx) = watcher(vec![file.clone()]);
        std::fs::write(&file, "a = 1\n").unwrap();
        std::fs::write(dir.join("other.toml"), "b = 1\n").unwrap();
        assert!(quiet(&rx));
    }

    #[test]
    fn sees_a_file_created_in_a_new_directory() {
        let dir = TempDir::new("watch-create");
        let file = dir.join("terminal-decline/themes/x.toml");
        let (w, rx) = watcher(vec![file.clone()]);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        assert!(quiet(&rx), "directories alone change nothing");
        std::fs::write(&file, "x").unwrap();
        rx.recv_timeout(WAIT).expect("new file not seen");
        std::fs::remove_file(&file).unwrap();
        rx.recv_timeout(WAIT).expect("deletion not seen");
        // A new list: its files are read without a report, then watched.
        let other = dir.join("other.toml");
        std::fs::write(&other, "1").unwrap();
        w.set_paths(vec![other.clone()]);
        assert!(quiet(&rx));
        std::fs::write(&other, "2").unwrap();
        rx.recv_timeout(WAIT).expect("file from set_paths not seen");
    }

    #[test]
    fn follows_a_symlink_to_its_target() {
        let dir = TempDir::new("watch-link");
        std::fs::create_dir_all(dir.join("dotfiles")).unwrap();
        std::fs::create_dir_all(dir.join("terminal-decline")).unwrap();
        let real = dir.join("dotfiles/config.toml");
        std::fs::write(&real, "a = 1\n").unwrap();
        let link = dir.join("terminal-decline/config.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let list = watch_list(std::slice::from_ref(&link));
        assert!(list.contains(&dir.join("dotfiles")), "{list:?}");
        assert!(list.contains(&dir.join("terminal-decline")), "{list:?}");
        let (_w, rx) = watcher(vec![link]);
        let tmp = dir.join("dotfiles/tmp");
        std::fs::write(&tmp, "a = 2\n").unwrap();
        std::fs::rename(&tmp, &real).unwrap();
        rx.recv_timeout(WAIT).expect("target replaced, not seen");
    }
}
