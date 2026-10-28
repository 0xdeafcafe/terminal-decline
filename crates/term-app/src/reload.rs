//! Dogfood builds restart into a newer build as soon as one is installed.
//!
//! `scripts/dogfood.sh` marks the bundle with `Contents/Resources/dogfood`.
//! Such a build watches its own executable with kqueue (a thread blocked in
//! the kernel: no timers, no wakeups while nothing changes). When the file is
//! replaced it saves each window's directory and frame, starts the new build
//! with `--restore <file>`, and quits.

use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SavedWindow {
    pub cwd: Option<String>,
    /// x, y, width, height in screen points.
    pub frame: [f64; 4],
}

/// The app bundle this binary runs from, if any.
pub fn own_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let contents = exe.parent()?.parent()?;
    (contents.file_name()? == "Contents").then(|| contents.parent().map(Path::to_path_buf))?
}

pub fn is_dogfood_bundle(bundle: &Path) -> bool {
    bundle.join("Contents/Resources/dogfood").exists()
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
    std::thread::Builder::new().name("build-watch".into()).spawn(move || {
        let mut event: libc::kevent = unsafe { std::mem::zeroed() };
        loop {
            let n = unsafe { libc::kevent(kq, std::ptr::null(), 0, &mut event, 1, std::ptr::null()) };
            if n > 0 {
                break;
            }
            if n < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                return;
            }
        }
        unsafe {
            libc::close(fd);
            libc::close(kq);
        }
        // Let the installer finish putting the new bundle in place.
        std::thread::sleep(Duration::from_millis(400));
        on_change();
    })?;
    Ok(())
}

fn restore_file() -> PathBuf {
    let base = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Library/Caches/decline/restore.json")
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
        let dir = std::env::temp_dir().join(format!("decline-reload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.json");
        let w = vec![SavedWindow {
            cwd: Some("/tmp".into()),
            frame: [1.0, 2.0, 300.0, 200.0],
        }];
        std::fs::write(&path, serde_json::to_vec(&w).unwrap()).unwrap();
        assert_eq!(take(&path), w);
        assert!(!path.exists());
        assert!(take(&path).is_empty());
    }

    #[test]
    fn watch_fires_when_the_file_is_replaced() {
        let dir = std::env::temp_dir().join(format!("decline-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("decline");
        std::fs::write(&exe, b"old").unwrap();
        let (tx, rx) = mpsc::channel();
        watch(exe.clone(), move || tx.send(()).unwrap()).unwrap();
        // How dogfood.sh installs: the old file goes away, a new one appears.
        std::fs::remove_file(&exe).unwrap();
        std::fs::write(&exe, b"new").unwrap();
        rx.recv_timeout(Duration::from_secs(5)).expect("no change seen");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
