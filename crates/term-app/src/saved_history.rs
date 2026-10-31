//! Each pane's history across a restart (`restore_scrollback`): written
//! when the app restarts into a new build (and, with `restore_scrollback =
//! true`, when it quits), one file per pane in
//! `~/Library/Application Support/terminal-decline/restore/`, named in
//! the saved window list ([`crate::reload::SavedPane::history`]), read
//! and deleted when the pane opens again.
//!
//! The files hold the panes' scrollback-v2 blocks, compressed as history
//! keeps them ([`term_core::Terminal::save_history`]): nothing is written
//! while the app runs, only as it goes. The folder is 0700 and each file
//! 0600, created new (never through a link left in its place). Nothing is
//! saved for a pane on the alternate screen (an editor, a pager, a
//! password manager) nor while Secure Keyboard Entry is on, whose point
//! is that what is typed stays in the app. Files no window list names
//! (the app was killed before it restored them, say) are deleted at the
//! next launch.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use term_core::Term;

/// A pane's file ends so.
const EXT: &str = "hist";
/// The window list a quit saves (`restore_scrollback = true`).
const SESSION: &str = "session.json";

/// The folder the files go in.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn in_dir(dir: PathBuf) -> Store {
        Store { dir }
    }

    /// `~/Library/Application Support/terminal-decline/restore`.
    pub fn current() -> Store {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        Store::in_dir(home.join("Library/Application Support/terminal-decline/restore"))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The window list a quit left, for the next launch.
    pub fn session(&self) -> PathBuf {
        self.dir.join(SESSION)
    }

    fn make_dir(&self) -> std::io::Result<()> {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&self.dir)?;
        // Made before by something else: tighten it.
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700))
    }

    /// Write `bytes` to a new file of its own, readable only by the user.
    /// Returns its name.
    fn write_new(&self, stem: &str, ext: &str, bytes: &[u8]) -> std::io::Result<String> {
        self.make_dir()?;
        let name = format!("{stem}.{ext}");
        let path = self.dir.join(&name);
        let _ = std::fs::remove_file(&path);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        f.write_all(bytes)?;
        Ok(name)
    }

    /// Save `term`'s history, its newest `limit` lines, for the pane to
    /// show again after the restart. None if there is nothing to save
    /// (the alternate screen, no lines) or it couldn't be written.
    pub fn save(&self, term: &Term, limit: usize) -> Option<String> {
        if limit == 0 {
            return None;
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let bytes = term.state.save_history(limit, now)?;
        static N: AtomicU32 = AtomicU32::new(0);
        let stem = format!("{}-{}-{}", std::process::id(), now, N.fetch_add(1, Ordering::Relaxed));
        match self.write_new(&stem, EXT, &bytes) {
            Ok(name) => Some(name),
            Err(e) => {
                eprintln!("terminal-decline: saving scrollback in {}: {e}", self.dir.display());
                None
            }
        }
    }

    /// Read and delete the file `name` (only a name this module gives:
    /// nothing outside the folder).
    pub fn take(&self, name: &str) -> Option<Vec<u8>> {
        if !valid_name(name) {
            return None;
        }
        let path = self.dir.join(name);
        let bytes = std::fs::read(&path).ok();
        let _ = std::fs::remove_file(&path);
        bytes
    }

    /// Put the history saved as `name` back into `term`, a new pane's
    /// terminal nothing has been written to, with a faint line saying
    /// when it is from below it. Whatever goes wrong, the pane opens as
    /// it would have without it.
    pub fn restore(&self, term: &mut Term, name: &str) {
        let Some(bytes) = self.take(name) else { return };
        match term.state.restore_history(&bytes) {
            Ok(r) if r.lines > 0 => {
                term.advance(format!("\x1b[0;2m{}\x1b[0m\r\n", restored_line(r.saved_at)).as_bytes());
                term.state.take_replies();
            }
            Ok(_) => {}
            Err(e) => eprintln!("terminal-decline: restoring scrollback: {e}"),
        }
    }

    /// Write the window list a quit saves.
    pub fn save_session(&self, json: &[u8]) -> std::io::Result<()> {
        self.write_new("session", "json", json).map(drop)
    }

    /// Delete pane files left over: older than a minute (a restart's are
    /// read within seconds), once the windows the launch reopens have
    /// taken theirs.
    pub fn clean_up(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        let old = SystemTime::now() - Duration::from_secs(60);
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !valid_name(&name) {
                continue;
            }
            if e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < old) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    /// Delete every pane file and the quit's window list.
    pub fn clear(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if valid_name(&name) || name == SESSION {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// A pane file's name: `pid-time-n.hist`, nothing else.
fn valid_name(name: &str) -> bool {
    name.strip_suffix(&format!(".{EXT}"))
        .is_some_and(|stem| !stem.is_empty() && stem.bytes().all(|b| b.is_ascii_digit() || b == b'-'))
}

/// `restored from Sat 31 Oct 14:05`, in local time.
pub fn restored_line(saved_at: u64) -> String {
    let secs = saved_at as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!(
        "restored from {} {} {} {:02}:{:02}",
        DAYS[tm.tm_wday.clamp(0, 6) as usize],
        tm.tm_mday,
        MONTHS[tm.tm_mon.clamp(0, 11) as usize],
        tm.tm_hour,
        tm.tm_min
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("td-saved-history-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store::in_dir(dir.join("restore"))
    }

    fn term_with_lines(n: usize) -> Term {
        let mut t = Term::new(40, 5, 10_000, term_core::ImagePool::process());
        for i in 0..n {
            t.advance(format!("\x1b[32mline {i}\x1b[0m\r\n").as_bytes());
        }
        t
    }

    #[test]
    fn saves_privately_and_restores_once() {
        let s = store("once");
        let t = term_with_lines(100);
        let name = s.save(&t, 10_000).expect("saved");
        assert!(valid_name(&name), "{name}");
        let path = s.dir().join(&name);
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(s.dir()), 0o700);
        let mut back = Term::new(40, 5, 10_000, term_core::ImagePool::process());
        s.restore(&mut back, &name);
        assert!(!path.exists(), "deleted once read");
        let dump = back.state.dump();
        assert!(dump.contains("line 99"), "{dump}");
        assert!(dump.contains("restored from "), "{dump}");
        assert_eq!(back.state.scrollback_len(), 100 - 3);
        // Again: nothing there, nothing restored.
        let mut again = Term::new(40, 5, 10_000, term_core::ImagePool::process());
        s.restore(&mut again, &name);
        assert_eq!(again.state.scrollback_len(), 0);
        let _ = std::fs::remove_dir_all(s.dir().parent().unwrap());
    }

    #[test]
    fn nothing_from_the_alternate_screen_or_with_no_limit() {
        let s = store("alt");
        let mut t = term_with_lines(10);
        assert!(s.save(&t, 0).is_none());
        t.advance(b"\x1b[?1049hsecret");
        assert!(s.save(&t, 10_000).is_none());
        assert!(!s.dir().exists() || std::fs::read_dir(s.dir()).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(s.dir().parent().unwrap());
    }

    #[test]
    fn names_outside_the_folder_are_refused() {
        let s = store("names");
        for bad in ["../x.hist", "/etc/passwd", "a.hist", ".hist", "1-2.json", "1/2.hist"] {
            assert!(s.take(bad).is_none(), "{bad}");
        }
        assert!(valid_name("123-456-0.hist"));
    }

    #[test]
    fn clean_up_keeps_recent_files() {
        let s = store("clean");
        let t = term_with_lines(5);
        let (a, b) = (s.save(&t, 100).unwrap(), s.save(&t, 100).unwrap());
        let old = SystemTime::now() - Duration::from_secs(3600);
        for n in [&a, &b] {
            std::fs::File::options()
                .write(true)
                .open(s.dir().join(n))
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        let c = s.save(&t, 100).unwrap();
        s.clean_up();
        assert!(!s.dir().join(&a).exists() && !s.dir().join(&b).exists(), "old");
        assert!(s.dir().join(&c).exists(), "recent");
        s.clear();
        assert!(std::fs::read_dir(s.dir()).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(s.dir().parent().unwrap());
    }

    #[test]
    fn the_restored_line_names_the_day_and_time() {
        let line = restored_line(1_700_000_000);
        assert!(line.starts_with("restored from "), "{line}");
        assert!(line.contains("Nov"), "{line}");
    }
}
