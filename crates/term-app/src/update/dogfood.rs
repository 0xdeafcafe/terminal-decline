//! The app's side of updates from source ([`super::source`]): the
//! one-shot timers, the checks and builds on their own threads, and the
//! notifications. Everything here but those threads runs on the main
//! thread. Only Terminal Decline.app's first process does any of it, in
//! a dogfood bundle whose marker names its checkout, and never under the
//! test control.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use dispatch2::{DispatchQueue, DispatchTime};
use objc2::MainThreadMarker;

use super::log;
use super::source::{self, Asked, Decision, Remote, Situation, Source, Tree};
use crate::config::UpdateMode;
use crate::entry::Role;
use crate::notify::{self, Button, Note, Target};

struct State {
    src: Source,
    /// The installed bundle.
    bundle: PathBuf,
    mode: UpdateMode,
    /// A check is under way.
    checking: bool,
    /// A build is under way.
    updating: bool,
    /// Bumped each time the schedule is armed: an older timer does
    /// nothing when it fires.
    armed: u64,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Held for the life of the process: one process checks.
static LOCK: std::sync::OnceLock<std::fs::File> = std::sync::OnceLock::new();

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

/// Start checking, if this is a dogfood build that knows its checkout.
/// Returns whether it does (the menu then has Check for Updates… and
/// Update to Latest).
pub fn start(role: &Role, mode: UpdateMode) -> bool {
    if *role != Role::App || std::env::var_os(crate::control::ENV).is_some_and(|v| !v.is_empty()) {
        return false;
    }
    let Some(bundle) = crate::reload::own_bundle().filter(|b| crate::reload::is_dogfood_bundle(b)) else {
        return false;
    };
    let Some(src) = source::of_bundle(&bundle) else {
        log("source: the dogfood marker names no checkout (built before updates from source)");
        return false;
    };
    let lock = home().join("Library/Caches/terminal-decline/source-updater.lock");
    if LOCK.get().is_none() {
        match super::take_lock(&lock) {
            Some(f) => {
                let _ = LOCK.set(f);
            }
            None => return false,
        }
    }
    log(&format!(
        "source: built {} from {}; update = {mode:?}",
        short(&src.commit),
        src.repo.display()
    ));
    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            src,
            bundle,
            mode,
            checking: false,
            updating: false,
            armed: 0,
        })
    });
    if mode.on() {
        arm(source::FIRST_CHECK);
    }
    true
}

/// The checkout and commit this build came from, while updates from
/// source run.
pub fn source() -> Option<Source> {
    STATE.with(|s| s.borrow().as_ref().map(|st| st.src.clone()))
}

/// Whether updates from source run in this process.
pub fn shown() -> bool {
    STATE.with(|s| s.borrow().is_some())
}

/// The `update` setting changed.
pub fn set_mode(mode: UpdateMode) {
    let changed = STATE.with(|s| match s.borrow_mut().as_mut() {
        Some(st) if st.mode != mode => {
            st.mode = mode;
            true
        }
        _ => false,
    });
    if changed {
        log(&format!("source: update = {mode:?}"));
        if mode.on() {
            arm(source::FIRST_CHECK);
        } else {
            // Any timer armed does nothing now.
            STATE.with(|s| {
                if let Some(st) = s.borrow_mut().as_mut() {
                    st.armed += 1;
                }
            });
        }
    }
}

/// Terminal Decline › Check for Updates….
pub fn check_now() {
    run(Asked::Check);
}

/// Terminal Decline › Update to Latest, or a notification's Update.
pub fn update_now() {
    run(Asked::Update);
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(9)]
}

/// The next scheduled check, `after` from now: one timer, re-armed after
/// each check.
fn arm(after: Duration) {
    let Some(n) = STATE.with(|s| {
        s.borrow_mut().as_mut().map(|st| {
            st.armed += 1;
            st.armed
        })
    }) else {
        return;
    };
    let at = DispatchTime::NOW.time(after.as_nanos().min(i64::MAX as u128) as i64);
    let _ = DispatchQueue::main().after(at, move || {
        if STATE.with(|s| s.borrow().as_ref().is_some_and(|st| st.armed == n)) {
            run(Asked::Nobody);
        }
    });
}

fn show(note: Note) {
    if let Some(mtm) = MainThreadMarker::new() {
        notify::show(mtm, note);
    }
}

/// A note about updates: they replace each other.
fn note(title: impl Into<String>, body: impl Into<String>) -> Note {
    let mut n = Note::new(title, body, Target::App);
    n.key = Some("source-update".into());
    n.group = Some("updates".into());
    n
}

fn run(asked: Asked) {
    let Some((src, busy)) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let st = s.as_mut()?;
        let busy = st.checking || st.updating;
        if !busy {
            st.checking = true;
        }
        Some((st.src.clone(), busy.then_some(st.updating)))
    }) else {
        return;
    };
    if let Some(updating) = busy {
        if asked != Asked::Nobody {
            let what = if updating { "An update is being built" } else { "Already checking" };
            show(note(what, "It says when it's done."));
        }
        return;
    }
    std::thread::spawn(move || {
        let remote = source::check(&src);
        let tree = source::tree(&src.repo);
        DispatchQueue::main().exec_async(move || checked(asked, remote, tree));
    });
}

fn checked(asked: Asked, remote: Result<Remote, String>, tree: Result<Tree, String>) {
    let Some((src, mode)) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let st = s.as_mut()?;
        st.checking = false;
        Some((st.src.clone(), st.mode))
    }) else {
        return;
    };
    let scheduled = asked == Asked::Nobody;
    let (remote, tree) = match (remote, tree) {
        (Ok(r), Ok(t)) => (r, t),
        (Err(e), _) | (_, Err(e)) => {
            log(&format!("source: check failed: {e}"));
            if !scheduled {
                show(note("Couldn't check for updates", e));
            } else {
                arm(source::CHECK_EVERY);
            }
            return;
        }
    };
    let told_file = source::told_file(&home());
    let told = std::fs::read_to_string(&told_file).is_ok_and(|t| t.trim() == remote.head);
    let situation = Situation {
        mode,
        asked,
        ahead: remote.ahead,
        tree,
        told,
        idle: source::idle(),
        on_ac: source::on_ac(),
        programs: !crate::app::running_programs().is_empty(),
    };
    let decision = source::decide(&situation);
    log(&format!(
        "source: {} commit(s) behind {} ({}); {decision:?}",
        remote.ahead,
        short(&remote.head),
        match asked {
            Asked::Nobody => "scheduled",
            Asked::Check => "Check for Updates",
            Asked::Update => "Update",
        }
    ));
    let mark_told = || {
        if let Some(dir) = told_file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&told_file, &remote.head);
    };
    let tell = |remote: &Remote| {
        let mut n = note(
            source::available_title(remote.ahead),
            source::subjects_text(&remote.subjects, remote.ahead),
        );
        n.button = Some(Button::Update);
        show(n);
    };
    let mut next = Some(source::CHECK_EVERY);
    match decision {
        Decision::Nothing => {}
        Decision::UpToDate => show(note(
            format!("{} is up to date", crate::PRODUCT_NAME),
            format!("Built from {}, the newest on {}.", short(&src.commit), source::BRANCH),
        )),
        Decision::Tell => {
            tell(&remote);
            mark_told();
        }
        Decision::Blocked(why) => {
            show(note(source::available_title(remote.ahead), format!("Not updating: {why}.")));
            mark_told();
        }
        Decision::Wait { tell: t } => {
            if t {
                tell(&remote);
                mark_told();
            }
            next = Some(source::AUTO_RETRY);
        }
        Decision::Update => {
            next = None;
            build(src, remote, asked == Asked::Update);
        }
    }
    if scheduled && let Some(after) = next {
        arm(after);
    }
}

/// Build the new commits in the background; the install restarts the
/// app ([`crate::reload`]): once no program runs, and, unless the user
/// asked for the update (`asked`), once nobody has used the app for a
/// while. A failure is a notification naming the log.
fn build(src: Source, remote: Remote, asked: bool) {
    crate::app::set_update_asked(asked);
    let Some(bundle) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let st = s.as_mut()?;
        st.updating = true;
        Some(st.bundle.clone())
    }) else {
        return;
    };
    let commits = if remote.ahead == 1 { "commit" } else { "commits" };
    show(note(
        format!("Updating {}", crate::PRODUCT_NAME),
        format!(
            "Building {} new {commits} in the background; it restarts in the same windows when done, once no program is running.",
            remote.ahead
        ),
    ));
    let install = bundle
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home().join("Applications"));
    let log_file = source::build_log(&home());
    std::thread::spawn(move || {
        let result = source::run_update(&src, &install, &log_file);
        DispatchQueue::main().exec_async(move || built(result, log_file));
    });
}

fn built(result: Result<(), String>, log_file: PathBuf) {
    STATE.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            st.updating = false;
        }
    });
    match result {
        // The new build is installed: the restart follows.
        Ok(()) => log("source: built and installed"),
        Err(e) => {
            crate::app::set_update_asked(false);
            log(&format!("source: update failed: {e}"));
            let shown = crate::activity::tilde(&log_file.to_string_lossy(), std::env::var("HOME").ok().as_deref());
            let mut n = note("Update failed", format!("{e}. The log: {shown}"));
            n.target = Target::Open(log_file.to_string_lossy().into_owned());
            show(n);
            arm(source::CHECK_EVERY);
        }
    }
}
