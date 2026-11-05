//! Updates for builds from source (dogfood builds, installed by
//! `scripts/dogfood.sh`; docs/design/updater.md §11).
//!
//! The bundle's `Contents/Resources/dogfood` marker names the checkout it
//! was built from and the commit ([`Source`]). On launch and then every
//! [`CHECK_EVERY`] (one-shot timers, re-armed after each check: no
//! polling), a thread fetches `origin/main` into that checkout and counts
//! the commits the build lacks, on GitHub's `main` or the checkout's own
//! (agent sessions merge into the checkout's `main` without always
//! pushing) ([`check`]). [`decide`] says what to do with what it found and
//! the `update` setting: tell (a notification with an Update button),
//! update (with `update = "auto"`, once the Mac has been idle a while, on
//! AC power, with no program running in any pane), or say why not
//! (uncommitted changes, another branch, a merge that conflicts).
//! Updating brings the checkout's `main` up to GitHub's (a fast-forward,
//! or a merge when both changed; never a rebase or a reset, and nothing is
//! pushed) and runs `scripts/dogfood.sh` in the
//! background at low priority (`taskpolicy -b`), its output in
//! `~/Library/Logs/terminal-decline/update-build.log`; the installed build
//! restarts the app as any dogfood install does ([`term_restore::reload`]). A
//! failed build is a notification naming the log.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::UpdateMode;

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

/// How the checkout's `main` and GitHub's (`origin/main`) stand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    /// The same commit.
    Same,
    /// The checkout has commits GitHub lacks, and there is nothing new to
    /// fetch.
    Ahead,
    /// GitHub has commits the checkout lacks: a fast-forward.
    Behind,
    /// Both have commits the other lacks: a merge.
    Diverged,
}

impl Relation {
    fn of(local_only: u32, remote_only: u32) -> Self {
        match (local_only > 0, remote_only > 0) {
            (false, false) => Relation::Same,
            (true, false) => Relation::Ahead,
            (false, true) => Relation::Behind,
            (true, true) => Relation::Diverged,
        }
    }
}

/// What a check found: the commits the build lacks, on the checkout's
/// `main` or GitHub's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    /// `origin/main`'s commit.
    pub head: String,
    /// The checkout's `main`.
    pub local: String,
    pub relation: Relation,
    /// Commits on either `main` the build hasn't.
    pub new: u32,
    /// The newest of those commits' subjects, newest first (at most 5).
    pub subjects: Vec<String>,
    /// When diverged: the files a merge of `origin/main` into the
    /// checkout's `main` would conflict in (worked out without touching
    /// the checkout). Empty when it merges cleanly.
    pub conflicts: Vec<String>,
}

impl Remote {
    /// What "told about this already" remembers: both `main`s.
    pub fn key(&self) -> String {
        format!("{} {}", self.local, self.head)
    }
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

/// A local `git` whose exit code is the answer (`merge-base
/// --is-ancestor`, `merge-tree`), with its output: `Err` only when it
/// didn't run.
fn git_code(repo: &Path, args: &[&str]) -> Result<(i32, String), String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("PATH", search_path())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    Ok((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
    ))
}

/// PATH for git and the build: a Dock launch has only the system's, so
/// Homebrew's and cargo's folders go first.
fn search_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let have = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".into());
    format!("{home}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:{have}")
}

/// Fetch `origin/main` and see what the checkout's `main` and GitHub's
/// have that `src.commit` hasn't, and how the two stand. Blocks on the
/// network: run it off the main thread.
pub fn check(src: &Source) -> Result<Remote, String> {
    git(&src.repo, &["fetch", "--quiet", "--no-tags", "origin", BRANCH])?;
    let remote = format!("refs/remotes/origin/{BRANCH}");
    let main = format!("refs/heads/{BRANCH}");
    let head = git(&src.repo, &["rev-parse", &remote])?;
    let local = git(&src.repo, &["rev-parse", &main])?;
    let sides = git(&src.repo, &["rev-list", "--left-right", "--count", &format!("{main}...{remote}")])?;
    let mut counts = sides.split_whitespace().map(str::parse::<u32>);
    let (Some(Ok(local_only)), Some(Ok(remote_only))) = (counts.next(), counts.next()) else {
        return Err(format!("git rev-list: {sides}"));
    };
    let relation = Relation::of(local_only, remote_only);
    let not_built = format!("^{}", src.commit);
    let new = git(&src.repo, &["rev-list", "--count", &main, &remote, &not_built])?
        .parse()
        .map_err(|e| format!("git rev-list: {e}"))?;
    let subjects = git(&src.repo, &["log", "--format=%s", "-n", "5", &main, &remote, &not_built])?
        .lines()
        .map(str::to_string)
        .collect();
    let conflicts = match relation {
        Relation::Diverged => merge_conflicts(&src.repo, &main, &remote),
        _ => Vec::new(),
    };
    Ok(Remote {
        head,
        local,
        relation,
        new,
        subjects,
        conflicts,
    })
}

/// The files merging `theirs` into `ours` would conflict in, worked out
/// in git's object store (`merge-tree`): the checkout isn't touched. A
/// git too old for `--write-tree` says nothing here; the update's own
/// merge finds out then.
fn merge_conflicts(repo: &Path, ours: &str, theirs: &str) -> Vec<String> {
    match git_code(repo, &["merge-tree", "--write-tree", "--name-only", "--no-messages", ours, theirs]) {
        // Exit 1 is "conflicts": the merged tree's id, then the files.
        Ok((1, out)) => out.lines().skip(1).filter(|l| !l.is_empty()).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

/// The files a merge under way conflicts in.
fn conflicted(repo: &Path) -> Vec<String> {
    git(repo, &["diff", "--name-only", "--diff-filter=U"])
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// "Your checkout and GitHub both changed …", naming the files.
pub fn conflict_text(files: &[String]) -> String {
    let mut text = "Your checkout and GitHub both changed and don't merge cleanly \u{2014} merge them by hand.".to_string();
    if !files.is_empty() {
        let shown: Vec<&str> = files.iter().take(5).map(String::as_str).collect();
        text.push_str(&format!("\nConflicts in {}", shown.join(", ")));
        if files.len() > shown.len() {
            text.push_str(&format!(" and {} more", files.len() - shown.len()));
        }
        text.push('.');
    }
    text
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

/// Every commit subject from `from` to `to`, oldest first and without
/// merges, for What's New when git-cliff didn't make the notes
/// ([`crate::whats_new::from_subjects`]). At most [`ALL_SUBJECTS`].
pub fn all_subjects_between(repo: &Path, from: &str, to: &str) -> Result<Vec<String>, String> {
    let range = format!("{from}..{to}");
    let n = ALL_SUBJECTS.to_string();
    let newest: Vec<String> = git(repo, &["log", "--no-merges", "--format=%s", "-n", &n, &range])?
        .lines()
        .map(str::to_string)
        .collect();
    Ok(newest.into_iter().rev().collect())
}

/// The most commits What's New reads from a checkout.
pub const ALL_SUBJECTS: usize = 500;

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
    /// Commits the build lacks ([`Remote::new`]).
    pub new: u32,
    pub tree: Tree,
    /// [`Remote::conflicts`].
    pub conflicts: Vec<String>,
    /// These two `main`s were already told about.
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
    /// "Update available", but merging GitHub's `main` into the
    /// checkout's would conflict in these files.
    Conflict(Vec<String>),
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
    if s.new == 0 {
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
    let quiet = s.asked == Asked::Nobody && s.told;
    if let Some(why) = blocked {
        return match quiet {
            true => Decision::Nothing,
            false => Decision::Blocked(why),
        };
    }
    if !s.conflicts.is_empty() {
        return match quiet {
            true => Decision::Nothing,
            false => Decision::Conflict(s.conflicts.clone()),
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

/// "Update available: 3 new commits", or, when only the checkout has
/// them (nothing new on GitHub), "Update to the checkout's latest: 3 new
/// commits".
pub fn available_title(relation: Relation, new: u32) -> String {
    let what = match relation {
        Relation::Ahead => "Update to the checkout's latest",
        _ => "Update available",
    };
    format!("{what}: {new} new commit{}", if new == 1 { "" } else { "s" })
}

/// The update notification's text: the newest subjects, and that the
/// update merges when both `main`s changed.
pub fn available_text(remote: &Remote) -> String {
    let mut text = subjects_text(&remote.subjects, remote.new);
    if remote.relation == Relation::Diverged {
        text.push_str("\nUpdating merges GitHub's main into the checkout's.");
    }
    text
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
    crate::logs_dir(home).join("update-build.log")
}

/// Why an update didn't happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// Merging GitHub's `main` into the checkout's conflicted in these
    /// files; the merge was aborted, the checkout is as it was.
    Conflict(Vec<String>),
    /// Anything else: a sentence for "Update failed".
    Other(String),
}

impl From<String> for Failure {
    fn from(e: String) -> Self {
        Failure::Other(e)
    }
}

/// Bring the checkout's `main` up to `origin/main` and run its
/// `scripts/dogfood.sh` at background priority, installing into
/// `install_dir`, with all output in `log`. Blocks for the whole build:
/// run it off the main thread. Checks the tree again first.
pub fn run_update(src: &Source, install_dir: &Path, log: &Path) -> Result<(), Failure> {
    run_update_at(src, install_dir, log, true)
}

/// Bring the checkout's `main` (checked out, clean) up to `origin/main`:
/// nothing when the checkout already has it all, a fast-forward when
/// only GitHub moved, a merge when both did. A merge that conflicts is
/// aborted. Never a rebase or a reset, and nothing is pushed.
fn bring_up(repo: &Path) -> Result<(), Failure> {
    let remote = format!("refs/remotes/origin/{BRANCH}");
    let has = |ancestor: &str, of: &str| git_code(repo, &["merge-base", "--is-ancestor", ancestor, of]).map(|(code, _)| code == 0);
    if has(&remote, "HEAD")? {
        return Ok(());
    }
    if has("HEAD", &remote)? {
        git(repo, &["merge", "--ff-only", "--quiet", &remote])?;
        return Ok(());
    }
    let Err(e) = git(repo, &["merge", "--no-edit", "--quiet", &remote]) else {
        return Ok(());
    };
    // Stopped part way (conflicts, or the merge commit failed) or refused
    // to start (an untracked file in the way): only the first leaves a
    // merge to abort.
    if git(repo, &["rev-parse", "--quiet", "--verify", "MERGE_HEAD"]).is_err() {
        return Err(Failure::Other(e));
    }
    let files = conflicted(repo);
    git(repo, &["merge", "--abort"])?;
    Err(match files.is_empty() {
        true => Failure::Other(e),
        false => Failure::Conflict(files),
    })
}

/// [`run_update`], with the build at background priority or not. The
/// tests turn it off: macOS starves a background process on a loaded Mac
/// (CPU and disk), which made a build of a two-line fake script take
/// a minute.
fn run_update_at(src: &Source, install_dir: &Path, log: &Path, background: bool) -> Result<(), Failure> {
    let t = tree(&src.repo)?;
    if !t.clean {
        return Err(Failure::Other("the checkout has uncommitted changes".into()));
    }
    if t.branch.as_deref() != Some(BRANCH) {
        return Err(Failure::Other(format!("the checkout isn't on {BRANCH}")));
    }
    bring_up(&src.repo)?;
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let file = std::fs::File::create(log).map_err(|e| format!("{}: {e}", log.display()))?;
    let err = file.try_clone().map_err(|e| e.to_string())?;
    let script = src.repo.join("scripts/dogfood.sh");
    // Background QoS: throttled CPU and disk, so the Mac stays quick.
    let taskpolicy = Path::new("/usr/sbin/taskpolicy");
    let mut cmd = if background && taskpolicy.exists() {
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
        Err(Failure::Other(format!("the build failed ({status})")))
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
    use std::ffi::{CStr, c_char, c_void};
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPSCopyPowerSourcesInfo() -> *const c_void;
        fn IOPSGetProvidingPowerSourceType(blob: *const c_void) -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
        fn CFStringGetCString(s: *const c_void, buf: *mut c_char, size: isize, encoding: u32) -> u8;
    }
    const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8
    unsafe {
        let blob = IOPSCopyPowerSourcesInfo();
        if blob.is_null() {
            return true;
        }
        // A CFString, not owned.
        let kind = IOPSGetProvidingPowerSourceType(blob);
        let mut buf = [0 as c_char; 64];
        let ac = kind.is_null()
            || (CFStringGetCString(kind, buf.as_mut_ptr(), buf.len() as isize, UTF8) != 0
                && CStr::from_ptr(buf.as_ptr()).to_bytes() == b"AC Power");
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
            new: 3,
            tree: Tree {
                clean: true,
                branch: Some("main".into()),
            },
            conflicts: Vec::new(),
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
            ("up to date", |s| s.new = 0, Nothing),
            (
                "up to date, asked",
                |s| {
                    s.new = 0;
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
            (
                "a merge would conflict",
                |s| s.conflicts = vec!["f".into()],
                Conflict(vec!["f".into()]),
            ),
            (
                "a merge would conflict, told",
                |s| {
                    s.conflicts = vec!["f".into()];
                    s.told = true;
                },
                Nothing,
            ),
            (
                "a merge would conflict, update asked",
                |s| {
                    s.conflicts = vec!["f".into()];
                    s.told = true;
                    s.asked = Asked::Update;
                },
                Conflict(vec!["f".into()]),
            ),
            (
                "a merge would conflict, auto and idle",
                |s| {
                    s.conflicts = vec!["f".into()];
                    s.mode = UpdateMode::Auto;
                    s.idle = AUTO_IDLE;
                },
                Conflict(vec!["f".into()]),
            ),
            (
                "dirty and would conflict: dirty first",
                |s| {
                    s.conflicts = vec!["f".into()];
                    s.tree.clean = false;
                },
                Blocked("the checkout has uncommitted changes".into()),
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
        assert_eq!(available_title(Relation::Behind, 1), "Update available: 1 new commit");
        assert_eq!(available_title(Relation::Diverged, 3), "Update available: 3 new commits");
        assert_eq!(
            available_title(Relation::Ahead, 2),
            "Update to the checkout's latest: 2 new commits"
        );
        let subj: Vec<String> = ["a", "b", "c", "d", "e"].map(String::from).to_vec();
        assert_eq!(subjects_text(&subj[..2], 2), "a\nb");
        assert_eq!(subjects_text(&subj, 12), "a\nb\nc\nand 9 more");
        assert_eq!(
            conflict_text(&[]),
            "Your checkout and GitHub both changed and don't merge cleanly \u{2014} merge them by hand."
        );
        let files: Vec<String> = ["a", "b", "c", "d", "e", "f", "g"].map(String::from).to_vec();
        assert!(conflict_text(&files[..2]).ends_with("\nConflicts in a, b."));
        assert!(conflict_text(&files).ends_with("\nConflicts in a, b, c, d, e and 2 more."));
    }

    /// A fake GitHub (a repo in a temp dir) and a clone of it as
    /// the checkout, built at the first commit. The update's own git runs
    /// with the user's config, so the clone carries an identity and no
    /// signing for the merge commit.
    struct Fake {
        dir: PathBuf,
        remote: PathBuf,
        work: PathBuf,
        src: Source,
        install: PathBuf,
        log: PathBuf,
    }

    impl Fake {
        fn new(name: &str) -> Fake {
            let dir = std::env::temp_dir().join(format!("td-source-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let remote = dir.join("remote");
            let work = dir.join("work");
            std::fs::create_dir_all(&remote).unwrap();
            let mut f = Fake {
                install: dir.join("Applications"),
                log: dir.join("logs/update-build.log"),
                src: Source {
                    repo: work.clone(),
                    commit: String::new(),
                },
                dir,
                remote,
                work,
            };
            f.at_remote(
                "git init -q -b main && mkdir scripts && \
                 printf '#!/bin/bash\\necho built \"$DECLINE_DOGFOOD_DIR\"\\n[ -e fail ] && exit 3\\nexit 0\\n' > scripts/dogfood.sh && \
                 echo one > f && git add . && git commit -q -m first",
            );
            sh(&f.dir, "git clone -q remote work");
            f.at_work("git config user.name t && git config user.email t@example.com && git config commit.gpgsign false");
            f.src.commit = f.at_work("git rev-parse HEAD");
            f
        }
        fn at_remote(&self, script: &str) -> String {
            sh(&self.remote, script)
        }
        fn at_work(&self, script: &str) -> String {
            sh(&self.work, script)
        }
        fn head(&self) -> String {
            self.at_work("git rev-parse HEAD")
        }
        fn update(&self) -> Result<(), Failure> {
            run_update_at(&self.src, &self.install, &self.log, false)
        }
        /// The build ran, with the install folder.
        fn built(&self) -> bool {
            std::fs::read_to_string(&self.log).is_ok_and(|t| t.trim() == format!("built {}", self.install.display()))
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn sh(cwd: &Path, script: &str) -> String {
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
    }

    /// Only GitHub moved: the check counts its commits, the update
    /// fast-forwards and runs the checkout's `scripts/dogfood.sh` with the
    /// install folder, logging its output; a failing build says so.
    #[test]
    fn behind_fast_forwards() {
        let f = Fake::new("behind");
        let r = check(&f.src).unwrap();
        assert_eq!((r.new, r.relation), (0, Relation::Same));
        assert_eq!(r.head, f.src.commit);
        f.at_remote("echo 1 > a && git add a && git commit -q -m 'add a' && echo 2 > b && git add b && git commit -q -m 'add b'");
        let r = check(&f.src).unwrap();
        assert_eq!((r.new, r.relation), (2, Relation::Behind));
        assert_eq!(r.subjects, ["add b", "add a"]);
        assert!(r.conflicts.is_empty());
        assert_eq!(subjects_between(&f.work, &f.src.commit, &r.head).unwrap(), ["add b", "add a"]);
        assert_eq!(
            all_subjects_between(&f.work, &f.src.commit, &r.head).unwrap(),
            ["add a", "add b"],
            "What's New reads them oldest first"
        );
        assert_eq!(
            tree(&f.work).unwrap(),
            Tree {
                clean: true,
                branch: Some("main".into())
            }
        );
        f.update().unwrap();
        assert_eq!(f.head(), r.head, "fast-forwarded");
        assert!(f.built());
        f.at_work("touch fail");
        assert_eq!(f.update().unwrap_err(), Failure::Other("the build failed (exit status: 3)".into()));
    }

    /// Only the checkout moved (an agent session merged without pushing):
    /// nothing to fetch, but a build older than it is offered the
    /// checkout's latest, built as it is.
    #[test]
    fn ahead_builds_the_checkout() {
        let f = Fake::new("ahead");
        f.at_work("echo 1 > a && git add a && git commit -q -m 'local a'");
        let local = f.head();
        let r = check(&f.src).unwrap();
        assert_eq!((r.new, r.relation), (1, Relation::Ahead));
        assert_eq!(r.subjects, ["local a"]);
        assert_eq!(r.local, local);
        assert_eq!(available_title(r.relation, r.new), "Update to the checkout's latest: 1 new commit");
        f.update().unwrap();
        assert_eq!(f.head(), local, "nothing merged");
        assert!(f.built());
        // Built at the checkout's latest: nothing new.
        let built = Source {
            repo: f.work.clone(),
            commit: local,
        };
        let r = check(&built).unwrap();
        assert_eq!((r.new, r.relation), (0, Relation::Ahead));
    }

    /// Both moved, in different files: GitHub's main is merged into the
    /// checkout's (a merge commit with both, nothing dropped, nothing
    /// pushed), then built.
    #[test]
    fn diverged_merges_cleanly() {
        let f = Fake::new("diverged");
        f.at_remote("echo 1 > a && git add a && git commit -q -m 'remote a'");
        let theirs = f.at_remote("git rev-parse HEAD");
        f.at_work("echo 2 > b && git add b && git commit -q -m 'local b'");
        let ours = f.head();
        let r = check(&f.src).unwrap();
        assert_eq!((r.new, r.relation), (2, Relation::Diverged));
        assert!(r.conflicts.is_empty());
        assert!(available_text(&r).ends_with("Updating merges GitHub's main into the checkout's."));
        f.update().unwrap();
        let parents = f.at_work("git log -1 --format=%P");
        assert_eq!(parents, format!("{ours} {theirs}"), "a merge of both");
        assert_eq!(f.at_work("git status --porcelain"), "");
        assert_eq!(f.at_work("cat a b"), "1\n2");
        assert_eq!(f.at_remote("git rev-parse HEAD"), theirs, "nothing pushed");
        assert!(f.built());
    }

    /// Both moved, in the same lines: the check names the file without
    /// touching the checkout, and the update's merge is aborted, leaving
    /// the checkout as it was and building nothing.
    #[test]
    fn diverged_conflict_is_aborted() {
        let f = Fake::new("conflict");
        f.at_remote("echo theirs > f && git commit -q -am 'remote f'");
        f.at_work("echo ours > f && git commit -q -am 'local f'");
        let ours = f.head();
        let r = check(&f.src).unwrap();
        assert_eq!((r.new, r.relation), (2, Relation::Diverged));
        assert_eq!(r.conflicts, ["f"]);
        assert_eq!(f.at_work("git status --porcelain"), "", "the check touches nothing");
        assert_eq!(f.update().unwrap_err(), Failure::Conflict(vec!["f".into()]));
        assert_eq!(f.head(), ours);
        assert_eq!(f.at_work("git status --porcelain"), "");
        assert_eq!(f.at_work("cat f"), "ours");
        assert!(!f.work.join(".git/MERGE_HEAD").exists(), "the merge was aborted");
        assert!(!f.log.exists(), "nothing built");
        assert_eq!(
            conflict_text(&r.conflicts),
            "Your checkout and GitHub both changed and don't merge cleanly \u{2014} merge them by hand.\nConflicts in f."
        );
    }

    /// Uncommitted changes to tracked files block the update.
    #[test]
    fn uncommitted_changes_block() {
        let f = Fake::new("uncommitted");
        f.at_remote("echo 1 > a && git add a && git commit -q -m 'add a'");
        check(&f.src).unwrap();
        f.at_work("echo x >> f");
        assert!(!tree(&f.work).unwrap().clean);
        assert_eq!(
            f.update().unwrap_err(),
            Failure::Other("the checkout has uncommitted changes".into())
        );
        assert_eq!(f.head(), f.src.commit);
        assert!(!f.built());
    }

    /// Untracked files never block it.
    #[test]
    fn untracked_files_dont_block() {
        let f = Fake::new("untracked");
        f.at_remote("echo 1 > a && git add a && git commit -q -m 'add a'");
        let theirs = f.at_remote("git rev-parse HEAD");
        check(&f.src).unwrap();
        f.at_work("touch untracked");
        assert!(tree(&f.work).unwrap().clean);
        f.update().unwrap();
        assert_eq!(f.head(), theirs);
        assert!(f.built());
    }

    /// A checkout on another branch isn't updated.
    #[test]
    fn another_branch_blocks() {
        let f = Fake::new("branch");
        f.at_remote("echo 1 > a && git add a && git commit -q -m 'add a'");
        check(&f.src).unwrap();
        f.at_work("git checkout -q -b other");
        assert_eq!(tree(&f.work).unwrap().branch.as_deref(), Some("other"));
        assert_eq!(f.update().unwrap_err(), Failure::Other("the checkout isn't on main".into()));
        assert!(!f.built());
    }
}
