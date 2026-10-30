//! Updates for builds from source (dogfood builds, installed by
//! `scripts/dogfood.sh`; docs/design/updater.md §11).
//!
//! The bundle's `Contents/Resources/dogfood` marker names the checkout it
//! was built from and the commit ([`Source`]). On launch and then every
//! [`CHECK_EVERY`] (one-shot timers, re-armed after each check: no
//! polling), a thread fetches `origin/main` into that checkout and counts
//! the commits the build is behind ([`check`]). [`decide`] says what to do
//! with what it found and the `update` setting: tell (a notification with
//! an Update button), update (with `update = "auto"`, once the Mac has
//! been idle a while, on AC power, with no program running in any pane),
//! or say why not (uncommitted changes, another branch). Updating
//! fast-forwards the checkout and runs `scripts/dogfood.sh` in the
//! background at low priority (`taskpolicy -b`), its output in
//! `~/Library/Logs/terminal-decline/update-build.log`; the installed build
//! restarts the app as any dogfood install does ([`crate::reload`]). A
//! failed build is a notification naming the log.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::config::UpdateMode;

/// The branch updates come from.
pub const BRANCH: &str = "main";
/// How often to check, at most.
pub const CHECK_EVERY: Duration = Duration::from_secs(4 * 3600);
/// The first check, after launch.
pub const FIRST_CHECK: Duration = Duration::from_secs(60);
/// With `update = "auto"` and an update waiting, how often to look again
/// whether the Mac is idle.
pub const AUTO_RETRY: Duration = Duration::from_secs(15 * 60);
/// How long nobody must have touched the Mac for an automatic update.
pub const AUTO_IDLE: Duration = Duration::from_secs(10 * 60);

/// Where a build from source came from: what `scripts/dogfood.sh` wrote
/// into the bundle's marker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    /// The checkout.
    pub repo: PathBuf,
    /// The commit built (full hash).
    pub commit: String,
}

/// The marker's text: `repo=…` and `commit=…` lines. An empty marker
/// (builds from before this) names no source.
pub fn parse_marker(text: &str) -> Option<Source> {
    let mut repo = None;
    let mut commit = None;
    for line in text.lines() {
        match line.split_once('=') {
            Some(("repo", v)) if !v.is_empty() => repo = Some(PathBuf::from(v)),
            Some(("commit", v)) if v.len() >= 7 && v.bytes().all(|b| b.is_ascii_hexdigit()) => commit = Some(v.to_string()),
            _ => {}
        }
    }
    Some(Source {
        repo: repo?,
        commit: commit?,
    })
}

/// The source of the dogfood build at `bundle`, if it says.
pub fn of_bundle(bundle: &Path) -> Option<Source> {
    let text = std::fs::read_to_string(bundle.join("Contents/Resources/dogfood")).ok()?;
    parse_marker(&text)
}

/// What a check found: how far `origin/main` is ahead of the build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    /// `origin/main`'s commit.
    pub head: String,
    /// Commits on it the build hasn't.
    pub ahead: u32,
    /// The newest of those commits' subjects, newest first (at most 5).
    pub subjects: Vec<String>,
}

/// The checkout as it is now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tree {
    /// No uncommitted changes (untracked files don't count).
    pub clean: bool,
    /// The branch checked out, `None` when detached.
    pub branch: Option<String>,
}

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        // Never ask for a password or a passphrase: fail instead.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env(
            "GIT_SSH_COMMAND",
            std::env::var("GIT_SSH_COMMAND").unwrap_or_else(|_| "ssh -o BatchMode=yes".into()),
        )
        .env("PATH", search_path())
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("git {}: {}", args.first().unwrap_or(&""), err.trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

/// PATH for git and the build: a Dock launch has only the system's, so
/// Homebrew's and cargo's folders go first.
fn search_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let have = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".into());
    format!("{home}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:{have}")
}

/// Fetch `origin/main` and see how far it is ahead of `src.commit`.
/// Blocks on the network: run it off the main thread.
pub fn check(src: &Source) -> Result<Remote, String> {
    git(&src.repo, &["fetch", "--quiet", "--no-tags", "origin", BRANCH])?;
    let remote = format!("origin/{BRANCH}");
    let head = git(&src.repo, &["rev-parse", &remote])?;
    let range = format!("{}..{remote}", src.commit);
    let ahead = git(&src.repo, &["rev-list", "--count", &range])?
        .parse()
        .map_err(|e| format!("git rev-list: {e}"))?;
    let subjects = git(&src.repo, &["log", "--format=%s", "-n", "5", &range])?
        .lines()
        .map(str::to_string)
        .collect();
    Ok(Remote { head, ahead, subjects })
}

/// The commits' subjects from `from` to `to`, newest first (at most 10),
/// for "Updated to …".
pub fn subjects_between(repo: &Path, from: &str, to: &str) -> Result<Vec<String>, String> {
    let range = format!("{from}..{to}");
    Ok(git(repo, &["log", "--format=%s", "-n", "10", &range])?
        .lines()
        .map(str::to_string)
        .collect())
}

/// Whether the checkout has uncommitted changes, and its branch.
pub fn tree(repo: &Path) -> Result<Tree, String> {
    let status = git(repo, &["status", "--porcelain", "--untracked-files=no"])?;
    let branch = git(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok();
    Ok(Tree {
        clean: status.is_empty(),
        branch,
    })
}

/// Who asked for this check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    /// The scheduled check.
    Nobody,
    /// Terminal Decline › Check for Updates….
    Check,
    /// Terminal Decline › Update to Latest, or a notification's Update.
    Update,
}

/// What [`decide`] looks at.
#[derive(Clone, Debug)]
pub struct Situation {
    pub mode: UpdateMode,
    pub asked: Asked,
    pub ahead: u32,
    pub tree: Tree,
    /// This `origin/main` was already told about.
    pub told: bool,
    /// Since anybody last touched the Mac.
    pub idle: Duration,
    pub on_ac: bool,
    /// A program other than a shell runs in some pane.
    pub programs: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Nothing,
    /// "Up to date" (only when asked).
    UpToDate,
    /// "Update available", with an Update button.
    Tell,
    /// "Update available", but it can't be installed: why.
    Blocked(String),
    /// Update now.
    Update,
    /// `update = "auto"`, not idle yet: look again in [`AUTO_RETRY`],
    /// telling first if not told yet.
    Wait {
        tell: bool,
    },
}

/// What to do about a check's result.
pub fn decide(s: &Situation) -> Decision {
    if s.ahead == 0 {
        return match s.asked {
            Asked::Nobody => Decision::Nothing,
            _ => Decision::UpToDate,
        };
    }
    if s.mode == UpdateMode::Off && s.asked == Asked::Nobody {
        return Decision::Nothing;
    }
    let blocked = if !s.tree.clean {
        Some("the checkout has uncommitted changes".to_string())
    } else if s.tree.branch.as_deref() != Some(BRANCH) {
        Some(match &s.tree.branch {
            Some(b) => format!("the checkout is on {b}, not {BRANCH}"),
            None => format!("the checkout isn't on a branch ({BRANCH} is updated)"),
        })
    } else {
        None
    };
    if let Some(why) = blocked {
        return match s.asked == Asked::Nobody && s.told {
            true => Decision::Nothing,
            false => Decision::Blocked(why),
        };
    }
    match (s.asked, s.mode) {
        (Asked::Update, _) => Decision::Update,
        (Asked::Check, _) => Decision::Tell,
        (Asked::Nobody, UpdateMode::Auto) => {
            if s.idle >= AUTO_IDLE && s.on_ac && !s.programs {
                Decision::Update
            } else {
                Decision::Wait { tell: !s.told }
            }
        }
        (Asked::Nobody, _) => match s.told {
            true => Decision::Nothing,
            false => Decision::Tell,
        },
    }
}

/// "Update available: 3 new commits".
pub fn available_title(ahead: u32) -> String {
    format!("Update available: {ahead} new commit{}", if ahead == 1 { "" } else { "s" })
}

/// The newest subjects, one per line (at most 3), and how many more.
pub fn subjects_text(subjects: &[String], ahead: u32) -> String {
    let shown: Vec<&str> = subjects.iter().take(3).map(String::as_str).collect();
    let more = ahead as usize - shown.len().min(ahead as usize);
    let mut text = shown.join("\n");
    if more > 0 {
        text.push_str(&format!("\nand {more} more"));
    }
    text
}

/// The build's log.
pub fn build_log(home: &Path) -> PathBuf {
    crate::crash::logs_dir(home).join("update-build.log")
}

/// Fast-forward the checkout to `origin/main` and run its
/// `scripts/dogfood.sh` at background priority, installing into
/// `install_dir`, with all output in `log`. Blocks for the whole build:
/// run it off the main thread. Checks the tree again first.
pub fn run_update(src: &Source, install_dir: &Path, log: &Path) -> Result<(), String> {
    let t = tree(&src.repo)?;
    if !t.clean {
        return Err("the checkout has uncommitted changes".into());
    }
    if t.branch.as_deref() != Some(BRANCH) {
        return Err(format!("the checkout isn't on {BRANCH}"));
    }
    git(&src.repo, &["merge", "--ff-only", "--quiet", &format!("origin/{BRANCH}")])?;
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let file = std::fs::File::create(log).map_err(|e| format!("{}: {e}", log.display()))?;
    let err = file.try_clone().map_err(|e| e.to_string())?;
    let script = src.repo.join("scripts/dogfood.sh");
    // Background QoS: throttled CPU and disk, so the Mac stays quick.
    let taskpolicy = Path::new("/usr/sbin/taskpolicy");
    let mut cmd = if taskpolicy.exists() {
        let mut c = Command::new(taskpolicy);
        c.arg("-b").arg("/bin/bash").arg(&script);
        c
    } else {
        let mut c = Command::new("/bin/bash");
        c.arg(&script);
        c
    };
    let status = cmd
        .current_dir(&src.repo)
        .env("DECLINE_DOGFOOD_DIR", install_dir)
        .env("PATH", search_path())
        .stdin(Stdio::null())
        .stdout(file)
        .stderr(err)
        .status()
        .map_err(|e| format!("{}: {e}", script.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("the build failed ({status})"))
    }
}

/// Where "told about this `origin/main`" is kept, so a relaunch doesn't
/// tell again.
pub fn told_file(home: &Path) -> PathBuf {
    home.join("Library/Caches/terminal-decline/update-told")
}

/// Seconds since anybody last touched the Mac (any input event).
pub fn idle() -> Duration {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event: u32) -> f64;
    }
    // kCGEventSourceStateCombinedSessionState, kCGAnyInputEventType.
    let s = unsafe { CGEventSourceSecondsSinceLastEventType(0, u32::MAX) };
    Duration::from_secs_f64(if s.is_finite() && s > 0.0 { s } else { 0.0 })
}

/// Whether the Mac runs on AC power (a desktop always does).
pub fn on_ac() -> bool {
    use objc2_foundation::NSString;
    use std::ffi::c_void;
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPSCopyPowerSourcesInfo() -> *const c_void;
        fn IOPSGetProvidingPowerSourceType(blob: *const c_void) -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }
    unsafe {
        let blob = IOPSCopyPowerSourcesInfo();
        if blob.is_null() {
            return true;
        }
        let kind = IOPSGetProvidingPowerSourceType(blob);
        // CFString and NSString are toll-free bridged; not owned.
        let ac = kind.is_null() || (*(kind as *const NSString)).to_string() == "AC Power";
        CFRelease(blob);
        ac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marker_names_the_source() {
        let s = parse_marker("repo=/Users/lw/src/td\ncommit=0123456789abcdef0123456789abcdef01234567\n").unwrap();
        assert_eq!(s.repo, PathBuf::from("/Users/lw/src/td"));
        assert_eq!(s.commit, "0123456789abcdef0123456789abcdef01234567");
        assert_eq!(parse_marker(""), None, "an old, empty marker");
        assert_eq!(parse_marker("repo=/x\n"), None);
        assert_eq!(parse_marker("repo=/x\ncommit=nothex!\n"), None);
    }

    fn situation() -> Situation {
        Situation {
            mode: UpdateMode::Notify,
            asked: Asked::Nobody,
            ahead: 3,
            tree: Tree {
                clean: true,
                branch: Some("main".into()),
            },
            told: false,
            idle: Duration::ZERO,
            on_ac: true,
            programs: false,
        }
    }

    #[test]
    fn decide_what_to_do() {
        use Decision::*;
        type Change = fn(&mut Situation);
        let rows: &[(&str, Change, Decision)] = &[
            ("notify tells", |_| {}, Tell),
            ("once", |s| s.told = true, Nothing),
            ("up to date", |s| s.ahead = 0, Nothing),
            (
                "up to date, asked",
                |s| {
                    s.ahead = 0;
                    s.asked = Asked::Check;
                },
                UpToDate,
            ),
            ("off", |s| s.mode = UpdateMode::Off, Nothing),
            (
                "off, but asked",
                |s| {
                    s.mode = UpdateMode::Off;
                    s.asked = Asked::Check;
                },
                Tell,
            ),
            (
                "asked again after being told",
                |s| {
                    s.told = true;
                    s.asked = Asked::Check;
                },
                Tell,
            ),
            ("update asked", |s| s.asked = Asked::Update, Update),
            (
                "dirty",
                |s| s.tree.clean = false,
                Blocked("the checkout has uncommitted changes".into()),
            ),
            (
                "dirty, told",
                |s| {
                    s.tree.clean = false;
                    s.told = true;
                },
                Nothing,
            ),
            (
                "dirty, update asked",
                |s| {
                    s.tree.clean = false;
                    s.told = true;
                    s.asked = Asked::Update;
                },
                Blocked("the checkout has uncommitted changes".into()),
            ),
            (
                "another branch",
                |s| s.tree.branch = Some("feat/x".into()),
                Blocked("the checkout is on feat/x, not main".into()),
            ),
            (
                "detached",
                |s| s.tree.branch = None,
                Blocked("the checkout isn't on a branch (main is updated)".into()),
            ),
            ("auto, busy", |s| s.mode = UpdateMode::Auto, Wait { tell: true }),
            (
                "auto, idle",
                |s| {
                    s.mode = UpdateMode::Auto;
                    s.idle = AUTO_IDLE;
                },
                Update,
            ),
            (
                "auto, idle on battery",
                |s| {
                    s.mode = UpdateMode::Auto;
                    s.idle = AUTO_IDLE;
                    s.on_ac = false;
                    s.told = true;
                },
                Wait { tell: false },
            ),
            (
                "auto, idle, a program runs",
                |s| {
                    s.mode = UpdateMode::Auto;
                    s.idle = AUTO_IDLE * 2;
                    s.programs = true;
                },
                Wait { tell: true },
            ),
            (
                "auto, dirty",
                |s| {
                    s.mode = UpdateMode::Auto;
                    s.idle = AUTO_IDLE;
                    s.tree.clean = false;
                },
                Blocked("the checkout has uncommitted changes".into()),
            ),
        ];
        for (name, change, want) in rows {
            let mut s = situation();
            change(&mut s);
            assert_eq!(&decide(&s), want, "{name}");
        }
    }

    #[test]
    fn notification_text() {
        assert_eq!(available_title(1), "Update available: 1 new commit");
        assert_eq!(available_title(3), "Update available: 3 new commits");
        let subj: Vec<String> = ["a", "b", "c", "d", "e"].map(String::from).to_vec();
        assert_eq!(subjects_text(&subj[..2], 2), "a\nb");
        assert_eq!(subjects_text(&subj, 12), "a\nb\nc\nand 9 more");
    }

    /// A checkout of a fake remote in a temp dir: the check sees new
    /// commits on the remote's main, the tree check sees changes and
    /// branches, and an update fast-forwards and runs the checkout's
    /// `scripts/dogfood.sh` with the install folder, logging its output.
    #[test]
    fn a_fake_remote() {
        let dir = std::env::temp_dir().join(format!("td-source-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let remote = dir.join("remote");
        let work = dir.join("work");
        std::fs::create_dir_all(&remote).unwrap();
        let sh = |cwd: &Path, script: &str| {
            let out = Command::new("/bin/sh")
                .arg("-c")
                .arg(script)
                .current_dir(cwd)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@example.com")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@example.com")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(out.status.success(), "{script}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        sh(
            &remote,
            "git init -q -b main && mkdir scripts && \
             printf '#!/bin/bash\\necho built \"$DECLINE_DOGFOOD_DIR\"\\n[ -e fail ] && exit 3\\nexit 0\\n' > scripts/dogfood.sh && \
             git add . && git commit -q -m first",
        );
        sh(&dir, "git clone -q remote work");
        let built = sh(&work, "git rev-parse HEAD");
        let src = Source {
            repo: work.clone(),
            commit: built.clone(),
        };
        let r = check(&src).unwrap();
        assert_eq!(r.ahead, 0);
        assert_eq!(r.head, built);
        sh(
            &remote,
            "echo 1 > a && git add a && git commit -q -m 'add a' && echo 2 > b && git add b && git commit -q -m 'add b'",
        );
        let r = check(&src).unwrap();
        assert_eq!(r.ahead, 2);
        assert_eq!(r.subjects, ["add b", "add a"]);
        assert_eq!(subjects_between(&work, &built, &r.head).unwrap(), ["add b", "add a"]);
        assert_eq!(
            tree(&work).unwrap(),
            Tree {
                clean: true,
                branch: Some("main".into())
            }
        );
        // Uncommitted changes: no update.
        sh(&work, "echo x >> scripts/dogfood.sh");
        assert!(!tree(&work).unwrap().clean);
        let install = dir.join("Applications");
        let log = dir.join("logs/update-build.log");
        assert!(run_update(&src, &install, &log).unwrap_err().contains("uncommitted"));
        sh(&work, "git checkout -q scripts/dogfood.sh");
        // Untracked files don't count.
        sh(&work, "touch untracked");
        assert!(tree(&work).unwrap().clean);
        // Another branch: no update.
        sh(&work, "git checkout -q -b other");
        assert_eq!(tree(&work).unwrap().branch.as_deref(), Some("other"));
        assert!(run_update(&src, &install, &log).is_err());
        sh(&work, "git checkout -q main");
        run_update(&src, &install, &log).unwrap();
        assert_eq!(sh(&work, "git rev-parse HEAD"), r.head, "fast-forwarded");
        let text = std::fs::read_to_string(&log).unwrap();
        assert_eq!(text.trim(), format!("built {}", install.display()));
        // A failing build says so.
        sh(&work, "touch fail");
        assert!(run_update(&src, &install, &log).unwrap_err().contains("build failed"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
