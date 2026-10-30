//! Telling about updates (docs/design/updater.md §10, question 10, and
//! §11): once Sparkle has an update waiting to install on quit, "Terminal
//! Decline X will install when you quit", once per version; and on the
//! first launch after any update, "Updated to X" with what changed.
//!
//! What was told and the build last launched are kept in
//! `~/Library/Caches/terminal-decline/updates-seen`, as `key=value` lines.

use std::path::{Path, PathBuf};

use super::log;
use crate::notify::{Note, Target};

/// What `updates-seen` remembers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    /// The release version last launched.
    pub version: Option<String>,
    /// The commit a build from source last launched was built from.
    pub commit: Option<String>,
    /// The version whose "will install when you quit" was told.
    pub ready: Option<String>,
}

impl Seen {
    pub fn parse(text: &str) -> Seen {
        let mut s = Seen::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = Some(v.to_string()).filter(|v| !v.is_empty());
            match k {
                "version" => s.version = v,
                "commit" => s.commit = v,
                "ready" => s.ready = v,
                _ => {}
            }
        }
        s
    }

    pub fn text(&self) -> String {
        let mut out = String::new();
        for (k, v) in [("version", &self.version), ("commit", &self.commit), ("ready", &self.ready)] {
            if let Some(v) = v {
                out.push_str(&format!("{k}={v}\n"));
            }
        }
        out
    }
}

/// The build running now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Build {
    /// A release, updated by Sparkle.
    Release(String),
    /// A build from source, at this commit.
    Source(String),
}

/// What changed since the last launch, if this build is newer than the
/// one launched before: the previous version or commit. Nothing the
/// first time (no record), or when the kind of build changed.
pub fn updated_from(seen: &Seen, now: &Build) -> Option<String> {
    match now {
        Build::Release(v) => seen.version.clone().filter(|old| old != v),
        Build::Source(c) => seen.commit.clone().filter(|old| old != c),
    }
}

/// `seen` with `now` recorded as the build last launched.
pub fn record(mut seen: Seen, now: &Build) -> Seen {
    match now {
        Build::Release(v) => seen.version = Some(v.clone()),
        Build::Source(c) => seen.commit = Some(c.clone()),
    }
    seen
}

/// The release page of version `v`.
pub fn release_page(v: &str) -> String {
    format!("https://github.com/0xdeafcafe/terminal-decline/releases/tag/v{v}")
}

/// "Updated to …": the title and text, `subjects` being the commits a
/// build from source gained (newest first).
pub fn updated_text(now: &Build, subjects: &[String]) -> (String, String) {
    let app = crate::PRODUCT_NAME;
    match now {
        Build::Release(v) => (format!("Updated to {app} {v}"), "The release notes say what's new.".into()),
        Build::Source(c) => {
            let short = &c[..c.len().min(9)];
            let n = subjects.len();
            let body = match n {
                0 => "Built from the newest commits.".to_string(),
                _ => super::source::subjects_text(subjects, n as u32),
            };
            (format!("Updated to {short}"), body)
        }
    }
}

/// "Terminal Decline X will install when you quit", if not told for this
/// version yet.
pub fn ready_text(seen: &Seen, version: &str) -> Option<String> {
    (seen.ready.as_deref() != Some(version)).then(|| format!("{} {version} will install when you quit", crate::PRODUCT_NAME))
}

fn seen_file() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
    Some(Path::new(&home).join("Library/Caches/terminal-decline/updates-seen"))
}

fn load() -> Seen {
    seen_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map_or_else(Seen::default, |t| Seen::parse(&t))
}

fn save(seen: &Seen) {
    // Tests never touch the real record.
    if std::env::var_os(crate::control::ENV).is_some_and(|v| !v.is_empty()) {
        return;
    }
    let Some(f) = seen_file() else { return };
    if let Some(dir) = f.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(f, seen.text());
}

fn show(note: Note) {
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        crate::notify::show(mtm, note);
    }
}

/// At launch, with an updater running (never under the test control):
/// "Updated to …" if this build is newer than the last one launched. For
/// a build from source the new commits' subjects are read from its
/// checkout, on a thread.
pub fn at_launch(now: Build, repo: Option<PathBuf>) {
    if std::env::var_os(crate::control::ENV).is_some_and(|v| !v.is_empty()) {
        return;
    }
    let seen = load();
    let from = updated_from(&seen, &now);
    save(&record(seen, &now));
    let Some(from) = from else { return };
    log(&format!("updated from {from} to {now:?}"));
    match (&now, repo) {
        (Build::Source(to), Some(repo)) => {
            let to = to.clone();
            std::thread::spawn(move || {
                let subjects = super::source::subjects_between(&repo, &from, &to).unwrap_or_default();
                dispatch2::DispatchQueue::main().exec_async(move || {
                    let (title, body) = updated_text(&Build::Source(to), &subjects);
                    show(Note::new(title, body, Target::App));
                });
            });
        }
        (Build::Release(v), _) => {
            let (title, body) = updated_text(&now, &[]);
            show(Note::new(title, body, Target::Open(release_page(v))));
        }
        (Build::Source(_), None) => {
            let (title, body) = updated_text(&now, &[]);
            show(Note::new(title, body, Target::App));
        }
    }
}

/// Sparkle has `version` downloaded, to install when the app quits: say
/// so, once per version.
pub fn ready(version: &str) {
    let mut seen = load();
    let Some(title) = ready_text(&seen, version) else { return };
    seen.ready = Some(version.to_string());
    save(&seen);
    let mut n = Note::new(title, "Nothing is interrupted: it installs the next time you quit.", Target::App);
    n.key = Some("sparkle-ready".into());
    n.group = Some("updates".into());
    show(n);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seen_reads_back() {
        let s = Seen {
            version: Some("0.5.0".into()),
            commit: None,
            ready: Some("0.6.0".into()),
        };
        assert_eq!(Seen::parse(&s.text()), s);
        assert_eq!(Seen::parse("junk\nversion=\n"), Seen::default());
    }

    #[test]
    fn updated_only_after_a_change() {
        let first = Seen::default();
        let rel = Build::Release("0.5.0".into());
        assert_eq!(updated_from(&first, &rel), None, "first launch");
        let seen = record(first, &rel);
        assert_eq!(updated_from(&seen, &rel), None, "same build");
        let newer = Build::Release("0.6.0".into());
        assert_eq!(updated_from(&seen, &newer).as_deref(), Some("0.5.0"));
        // A build from source keeps its own record.
        let src = Build::Source("abcdef0123".into());
        assert_eq!(updated_from(&seen, &src), None);
        let seen = record(seen, &src);
        assert_eq!(seen.version.as_deref(), Some("0.5.0"));
        assert_eq!(
            updated_from(&seen, &Build::Source("1234567890".into())).as_deref(),
            Some("abcdef0123")
        );
    }

    #[test]
    fn what_it_says() {
        let (t, b) = updated_text(&Build::Release("0.6.0".into()), &[]);
        assert_eq!(t, "Updated to Terminal Decline 0.6.0");
        assert!(b.contains("release notes"));
        assert_eq!(
            release_page("0.6.0"),
            "https://github.com/0xdeafcafe/terminal-decline/releases/tag/v0.6.0"
        );
        let subj: Vec<String> = ["b", "a"].map(String::from).to_vec();
        let (t, b) = updated_text(&Build::Source("0123456789abcdef".into()), &subj);
        assert_eq!(t, "Updated to 012345678");
        assert_eq!(b, "b\na");
        let mut seen = Seen::default();
        assert_eq!(
            ready_text(&seen, "0.6.0").unwrap(),
            "Terminal Decline 0.6.0 will install when you quit"
        );
        seen.ready = Some("0.6.0".into());
        assert_eq!(ready_text(&seen, "0.6.0"), None, "once");
        assert!(ready_text(&seen, "0.7.0").is_some());
    }
}
