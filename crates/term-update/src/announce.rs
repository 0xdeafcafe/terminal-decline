//! Telling about updates (docs/design/updater.md §10, question 10, §11
//! and §12): once Sparkle has an update waiting to install on quit,
//! "Terminal Decline X will install when you quit", once per version; and
//! on the first launch after any update, "Updated to X" with what changed
//! ([`super::whats_new`]), which a click (or Terminal Decline › What's
//! New) shows in full.
//!
//! What was told, the build last launched and the one it was updated
//! from are kept in `~/Library/Caches/terminal-decline/updates-seen`, as
//! `key=value` lines.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use term_activity::PRODUCT_NAME;

use super::log;
use crate::whats_new::{self, Notes};
use crate::{Notice, show};

/// What `updates-seen` remembers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    /// The release version last launched.
    pub version: Option<String>,
    /// The commit a build from source last launched was built from.
    pub commit: Option<String>,
    /// The version whose "will install when you quit" was told.
    pub ready: Option<String>,
    /// The release `version` was updated from: What's New lists the
    /// changes since it.
    pub version_from: Option<String>,
    /// The commit `commit` was updated from.
    pub commit_from: Option<String>,
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
                "version_from" => s.version_from = v,
                "commit_from" => s.commit_from = v,
                _ => {}
            }
        }
        s
    }

    pub fn text(&self) -> String {
        let mut out = String::new();
        for (k, v) in [
            ("version", &self.version),
            ("commit", &self.commit),
            ("ready", &self.ready),
            ("version_from", &self.version_from),
            ("commit_from", &self.commit_from),
        ] {
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

/// At a launch of `now`: the record to keep, and the build it was updated
/// from if this is the first launch since an update (the one time
/// "Updated to …" is told). The build updated from stays in the record,
/// for What's New, until the next update.
pub fn launched(seen: Seen, now: &Build) -> (Seen, Option<String>) {
    let from = updated_from(&seen, now);
    let mut seen = record(seen, now);
    if let Some(f) = &from {
        match now {
            Build::Release(_) => seen.version_from = Some(f.clone()),
            Build::Source(_) => seen.commit_from = Some(f.clone()),
        }
    }
    (seen, from)
}

/// The build `now` was last updated from, as recorded.
pub fn updated_last_from(seen: &Seen, now: &Build) -> Option<String> {
    match now {
        Build::Release(_) => seen.version_from.clone(),
        Build::Source(_) => seen.commit_from.clone(),
    }
}

/// The release page of version `v`.
pub fn release_page(v: &str) -> String {
    format!("https://github.com/0xdeafcafe/terminal-decline/releases/tag/v{v}")
}

/// "Updated to …": the title and text, with what changed if the notes
/// say ([`Notes::summary`]).
pub fn updated_text(now: &Build, notes: &Notes) -> (String, String) {
    let app = PRODUCT_NAME;
    let title = match now {
        Build::Release(v) => format!("Updated to {app} {v}"),
        Build::Source(c) => format!("Updated to {}", short(c)),
    };
    let body = match now {
        _ if !notes.is_empty() => notes.summary(),
        Build::Release(_) => "The release notes say what's new.".into(),
        Build::Source(_) => "Built from the newest commits.".into(),
    };
    (title, body)
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(9)]
}

/// Where a bundle keeps the notes it was built with.
pub fn notes_file(bundle: &Path) -> PathBuf {
    bundle.join("Contents/Resources/whats-new.md")
}

/// The notes of an update to `now` from `from` (`None`: no update
/// recorded). A release's are the versions it brought, from the notes in
/// `bundle`. A build from source's are the bundle's when they start at
/// `from` (dogfood.sh picked that range), else the commits from `from`
/// in its checkout `repo`, grouped the same way; with no `from`, the
/// bundle's. Reads files and runs git: not on the main thread.
pub fn notes(now: &Build, from: Option<&str>, bundle: Option<&Path>, repo: Option<&Path>) -> Notes {
    let embedded = bundle
        .and_then(|b| std::fs::read_to_string(notes_file(b)).ok())
        .map(|t| whats_new::parse(&t))
        .unwrap_or_default();
    match now {
        Build::Release(v) => embedded.between(from, v),
        Build::Source(to) => match (from, repo) {
            (Some(f), _) if embedded.from.as_deref() == Some(f) && !embedded.is_empty() => embedded,
            (Some(f), Some(repo)) => {
                let subjects = super::source::all_subjects_between(repo, f, to).unwrap_or_default();
                let mut n = whats_new::from_subjects(&subjects);
                n.from = Some(f.to_string());
                n.to = Some(to.clone());
                n
            }
            _ => embedded,
        },
    }
}

/// The What's New page: the notes, under a title naming the build and
/// what it was updated from.
pub fn page_text(now: &Build, from: Option<&str>, notes: &Notes, repo: Option<&Path>) -> String {
    let app = PRODUCT_NAME;
    let close = "q closes this tab".to_string();
    match now {
        Build::Release(v) => {
            let under: Vec<String> = from.map(|f| format!("Changes since {f}")).into_iter().collect();
            whats_new::page(
                &format!("What's New in {app} {v}"),
                &under,
                notes,
                &[format!("The release notes: {}", release_page(v)), close],
            )
        }
        Build::Source(c) => {
            let built = match from {
                Some(f) => format!("Built from {}; changes since {}", short(c), short(f)),
                None => format!("Built from {}", short(c)),
            };
            let footer: Vec<String> = repo
                .map(|r| {
                    let r = term_activity::activity::tilde(&r.to_string_lossy(), std::env::var("HOME").ok().as_deref());
                    format!("From the checkout in {r}")
                })
                .into_iter()
                .chain([close])
                .collect();
            whats_new::page(&format!("What's New in {app}"), &[built], notes, &footer)
        }
    }
}

/// "Terminal Decline X will install when you quit", if not told for this
/// version yet.
pub fn ready_text(seen: &Seen, version: &str) -> Option<String> {
    (seen.ready.as_deref() != Some(version)).then(|| format!("{} {version} will install when you quit", PRODUCT_NAME))
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
    if crate::under_test() {
        return;
    }
    let Some(f) = seen_file() else { return };
    if let Some(dir) = f.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(f, seen.text());
}

/// The build running, once [`at_launch`] has run: What's New is for it.
static CURRENT: OnceLock<(Build, Option<PathBuf>)> = OnceLock::new();

/// The build running and its checkout, if an updater runs (the menu then
/// has What's New).
pub fn current() -> Option<&'static (Build, Option<PathBuf>)> {
    CURRENT.get()
}

/// At launch, with an updater running (never under the test control):
/// "Updated to …" if this build is newer than the last one launched,
/// once. Its notes are read on a thread; clicking it shows What's New.
pub fn at_launch(now: Build, repo: Option<PathBuf>) {
    if crate::under_test() {
        return;
    }
    let _ = CURRENT.set((now.clone(), repo.clone()));
    let (seen, from) = launched(load(), &now);
    // Recorded before anything is shown: a crash or a restart now
    // doesn't tell it again.
    save(&seen);
    let Some(from) = from else { return };
    log(&format!("updated from {from} to {now:?}"));
    let bundle = term_restore::reload::own_bundle();
    std::thread::spawn(move || {
        let notes = notes(&now, Some(from.as_str()), bundle.as_deref(), repo.as_deref());
        dispatch2::DispatchQueue::main().exec_async(move || {
            let (title, body) = updated_text(&now, &notes);
            let mut n = Notice::new(title, body);
            n.key = Some("updated".into());
            n.group = Some("updates".into());
            if !notes.is_empty() {
                n.whats_new = true;
            } else if let Build::Release(v) = &now {
                n.open = Some(release_page(v));
            }
            show(n);
        });
    });
}

/// What's New for the build running: its notes since the build it was
/// last updated from, written as a page for `less -R`. Returns the
/// page's file. Reads files and runs git: not on the main thread.
pub fn whats_new_page() -> Option<PathBuf> {
    let (now, repo) = current()?;
    let from = updated_last_from(&load(), now);
    let bundle = term_restore::reload::own_bundle();
    let notes = notes(now, from.as_deref(), bundle.as_deref(), repo.as_deref());
    let text = page_text(now, from.as_deref(), &notes, repo.as_deref());
    let file = seen_file()?.with_file_name("whats-new.txt");
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&file, text).ok()?;
    Some(file)
}

/// The command a What's New tab runs for `page`: a pager that keeps its
/// colours; `q` ends it and closes the tab.
pub fn pager_argv(page: &Path) -> Vec<String> {
    vec![
        "/usr/bin/env".into(),
        "LESSCHARSET=utf-8".into(),
        "/usr/bin/less".into(),
        "-R".into(),
        "-PsWhat's New - q closes this tab".into(),
        page.to_string_lossy().into_owned(),
    ]
}

/// Sparkle has `version` downloaded, to install when the app quits: say
/// so, once per version.
pub fn ready(version: &str) {
    let mut seen = load();
    let Some(title) = ready_text(&seen, version) else { return };
    seen.ready = Some(version.to_string());
    save(&seen);
    let mut n = Notice::new(title, "Nothing is interrupted: it installs the next time you quit.");
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
            version_from: Some("0.4.0".into()),
            commit_from: Some("abc".into()),
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

    /// "Updated to …" once per update, across restarts; What's New keeps
    /// the build updated from until the next update.
    #[test]
    fn told_once_and_remembered() {
        for (old, new) in [
            (Build::Release("0.5.0".into()), Build::Release("0.6.0".into())),
            (Build::Source("aaaa".into()), Build::Source("bbbb".into())),
        ] {
            let (seen, from) = launched(Seen::default(), &old);
            assert_eq!(from, None, "first launch: nothing to tell");
            assert_eq!(updated_last_from(&seen, &old), None);
            let (seen, from) = launched(seen, &new);
            let was = match &old {
                Build::Release(v) | Build::Source(v) => v.clone(),
            };
            assert_eq!(from.as_deref(), Some(was.as_str()), "the update is told");
            // The restart into it, or any launch after: told already.
            let (seen, again) = launched(Seen::parse(&seen.text()), &new);
            assert_eq!(again, None, "not twice");
            assert_eq!(
                updated_last_from(&seen, &new).as_deref(),
                Some(was.as_str()),
                "What's New still knows"
            );
        }
    }

    /// A bundle in a temp dir, with these notes.
    struct Bundle(PathBuf);

    impl Bundle {
        fn new(name: &str, notes: Option<&str>) -> Bundle {
            let dir = std::env::temp_dir().join(format!("td-announce-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("Contents/Resources")).unwrap();
            if let Some(n) = notes {
                std::fs::write(notes_file(&dir), n).unwrap();
            }
            Bundle(dir)
        }
    }

    impl Drop for Bundle {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_notes_of_an_update() {
        let rel = Bundle::new(
            "rel",
            Some("## 0.6.0 - d6\n### Fixes\n- six\n## 0.5.0 - d5\n### Features\n- five\n## 0.4.0 - d4\n### Fixes\n- four\n"),
        );
        let now = Build::Release("0.6.0".into());
        let versions = |n: Notes| -> Vec<String> { n.sections.into_iter().filter_map(|s| s.version).collect() };
        assert_eq!(
            versions(notes(&now, Some("0.4.0"), Some(rel.0.as_path()), None)),
            ["0.6.0", "0.5.0"]
        );
        assert_eq!(
            versions(notes(&now, None, Some(rel.0.as_path()), None)),
            ["0.6.0"],
            "the menu, no update recorded"
        );
        assert!(notes(&now, Some("0.4.0"), None, None).is_empty(), "no bundle");

        // A dogfood build's notes start where dogfood.sh's range did.
        let src = Bundle::new(
            "src",
            Some("<!-- whats-new from=aaaa to=bbbb -->\n## Unreleased\n### Fixes\n- **app:** x\n"),
        );
        let now = Build::Source("bbbb".into());
        let n = notes(&now, Some("aaaa"), Some(src.0.as_path()), None);
        assert_eq!(n.summary(), "1 fix.\napp: x");
        assert_eq!(notes(&now, None, Some(src.0.as_path()), None), n, "the menu, no update recorded");
        // From another commit, with no checkout to read: the bundle's.
        assert_eq!(notes(&now, Some("cccc"), Some(src.0.as_path()), None), n);
    }

    #[test]
    fn what_it_says() {
        let none = Notes::default();
        let (t, b) = updated_text(&Build::Release("0.6.0".into()), &none);
        assert_eq!(t, "Updated to Terminal Decline 0.6.0");
        assert_eq!(b, "The release notes say what's new.");
        assert_eq!(
            release_page("0.6.0"),
            "https://github.com/0xdeafcafe/terminal-decline/releases/tag/v0.6.0"
        );
        let (t, b) = updated_text(&Build::Source("0123456789abcdef".into()), &none);
        assert_eq!(t, "Updated to 012345678");
        assert_eq!(b, "Built from the newest commits.");
        let some = whats_new::parse("### Features\n- **app:** a\n### Fixes\n- **app:** b\n");
        let (_, b) = updated_text(&Build::Source("0123456789abcdef".into()), &some);
        assert_eq!(b, "1 feature and 1 fix.\napp: a\napp: b");
        let mut seen = Seen::default();
        assert_eq!(
            ready_text(&seen, "0.6.0").unwrap(),
            "Terminal Decline 0.6.0 will install when you quit"
        );
        seen.ready = Some("0.6.0".into());
        assert_eq!(ready_text(&seen, "0.6.0"), None, "once");
        assert!(ready_text(&seen, "0.7.0").is_some());
    }

    #[test]
    fn the_page_names_the_builds() {
        let n = whats_new::parse("### Fixes\n- **app:** b\n");
        let p = page_text(&Build::Release("0.6.0".into()), Some("0.5.0"), &n, None);
        assert!(p.contains("What's New in Terminal Decline 0.6.0"), "{p}");
        assert!(p.contains("Changes since 0.5.0"), "{p}");
        assert!(p.contains("releases/tag/v0.6.0"), "{p}");
        let p = page_text(&Build::Source("0123456789abcdef".into()), Some("fedcba9876543210"), &n, None);
        assert!(p.contains("What's New in Terminal Decline\x1b"), "{p}");
        assert!(p.contains("Built from 012345678; changes since fedcba987"), "{p}");
        assert_eq!(pager_argv(Path::new("/tmp/w.txt")).last().map(String::as_str), Some("/tmp/w.txt"));
    }
}
