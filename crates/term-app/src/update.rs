//! Updates through Sparkle 2, in Terminal Decline.app's own process only
//! (docs/design/updater.md).
//!
//! The decisions are plain functions, tested here without Sparkle:
//! [`decide`] (whether this process runs the updater at all) and
//! [`answer`] (what the silent driver replies to each of Sparkle's
//! calls). [`sparkle`] loads Sparkle.framework at run time, never in a
//! pin, and forwards Sparkle's calls to them.
//!
//! Sparkle is off in dogfood builds, nightlies, ad-hoc builds and pins,
//! so a local build never installs a release. Dogfood builds update from
//! their own checkout instead ([`source`], [`dogfood`]).

use std::cell::RefCell;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::entry::Role;

pub mod dogfood;
pub mod source;
pub mod sparkle;

/// The test feed's env var, honoured only with the test control on
/// ([`crate::control::ENV`]).
pub const TEST_FEED_ENV: &str = "TD_UPDATE_FEED";

/// What [`decide`] looks at, gathered at launch.
#[derive(Clone, Debug)]
pub struct Context {
    pub role: Role,
    /// This process holds the updater lock ([`take_lock`]).
    pub primary: bool,
    /// `NSBundle.mainBundle`'s.
    pub bundle_id: Option<String>,
    /// From our own code signature; `None` when ad-hoc.
    pub team_id: Option<String>,
    /// `Contents/Resources/dogfood` exists ([`crate::reload`]).
    pub dogfood: bool,
    /// `CFBundleShortVersionString`.
    pub version: String,
    /// `update = true` (the default).
    pub setting: bool,
    /// `SUPublicEDKey` is in Info.plist and is 32 bytes of base64.
    pub public_key: bool,
    /// `Contents/Frameworks/Sparkle.framework` exists.
    pub framework: bool,
    /// The test control is on (`DECLINE_TEST_DUMP`).
    pub test: bool,
    /// With the test control on, `TD_UPDATE_FEED` is set.
    pub test_feed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    On,
    /// Off, and why (for `update.log`).
    Off(&'static str),
}

/// Whether this process runs the updater.
pub fn decide(c: &Context) -> Decision {
    use Decision::Off;
    match c.role {
        Role::Pin(_) => return Off("a pin (pins never update themselves)"),
        Role::Bare => return Off("not Terminal Decline.app (the bare binary)"),
        Role::App => {}
    }
    if !c.primary {
        return Off("another Terminal Decline process runs the updater");
    }
    if c.bundle_id.as_deref() != Some(crate::BUNDLE_ID) {
        return Off("the bundle id is not red.forbes.terminal-decline");
    }
    if c.dogfood {
        return Off("a dogfood build");
    }
    if c.version.contains("-nightly") || c.version.contains("-dev") {
        return Off("a nightly or dev build");
    }
    if c.test && !c.test_feed {
        return Off("the test control is on without a test feed");
    }
    if c.team_id.is_none() && !c.test_feed {
        return Off("an ad-hoc build (no team ID)");
    }
    if !c.setting {
        return Off("update = false");
    }
    if !c.public_key {
        return Off("no SUPublicEDKey in Info.plist");
    }
    if !c.framework {
        return Off("no Sparkle.framework in the bundle");
    }
    Decision::On
}

/// Whether `key` is base64 of 32 bytes, as an Ed25519 public key is.
pub fn is_public_key(key: &str) -> bool {
    base64_len(key.trim()) == Some(32)
}

/// The decoded length of standard, padded base64, if it is that.
fn base64_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.is_empty() || !b.len().is_multiple_of(4) {
        return None;
    }
    let pad = b.iter().rev().take_while(|&&c| c == b'=').count();
    if pad > 2 {
        return None;
    }
    let body = &b[..b.len() - pad];
    body.iter()
        .all(|c| c.is_ascii_alphanumeric() || *c == b'+' || *c == b'/')
        .then_some(b.len() / 4 * 3 - pad)
}

/// Where the updater lock lives: one updater per user, whatever the
/// number of app processes (`terminal-decline -e …` starts another).
pub fn lock_path(home: &Path) -> PathBuf {
    home.join("Library/Caches/terminal-decline/updater.lock")
}

/// Take the updater lock at `path`: an exclusive `flock`, held as long as
/// the file is open. `None` if another process (or open file) holds it.
pub fn take_lock(path: &Path) -> Option<File> {
    use std::os::fd::AsRawFd;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok()?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .ok()?;
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
    locked.then_some(file)
}

/// Sparkle's stage of an update (`SPUUserUpdateStage`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    NotDownloaded,
    Downloaded,
    /// Staged: it installs when the app quits.
    Installing,
}

impl Stage {
    pub fn from_raw(n: isize) -> Stage {
        match n {
            0 => Stage::NotDownloaded,
            1 => Stage::Downloaded,
            _ => Stage::Installing,
        }
    }
}

/// What the appcast item says about itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub information_only: bool,
    pub critical: bool,
    pub major_upgrade: bool,
}

/// Sparkle's calls to the user driver (`SPUUserDriver`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Call {
    PermissionRequest,
    UserInitiatedCheck,
    UpdateFound(Item, Stage),
    ReleaseNotes,
    UpdateNotFound,
    UpdaterError,
    /// Download and extraction progress.
    Progress,
    ReadyToInstall,
    Installing,
    InstalledAndRelaunched,
    InFocus,
    DismissInstallation,
}

/// `SPUUserUpdateChoice`. Skip is never chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Install = 1,
    Dismiss = 2,
}

/// What the driver does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Nothing,
    /// Reply to the permission request: automatic checks as the setting
    /// says, no system profile.
    Permission {
        automatic_checks: bool,
    },
    /// Reply at once.
    Choose(Choice),
    /// The "Quit and Install" alert: Install, or Dismiss for Later.
    QuitAndInstall,
    /// An alert with a link to the release page, then Dismiss.
    InformationOnly,
    /// "You're up to date", then acknowledge.
    UpToDate,
    /// The error in an alert, then acknowledge.
    ShowError,
    Acknowledge,
    /// Bring our alert forward, if one is open.
    Focus,
    /// Close our alert, if one is open: the session ended.
    CloseAlert,
}

/// What the driver answers `call`, in a session the user started or
/// not, with the `update` setting at `setting` (docs/design/updater.md
/// §3.2). Background checks never show anything, except the "Quit and
/// Install" alert for an update that has waited (Sparkle's impatient
/// interval, a week): with a normal update Sparkle downloads and stages it
/// without calling the driver, and it installs on quit.
pub fn answer(call: Call, user: bool, setting: bool) -> Answer {
    use Answer::*;
    match call {
        Call::PermissionRequest => Permission { automatic_checks: setting },
        Call::UserInitiatedCheck | Call::ReleaseNotes | Call::Progress | Call::Installing => Nothing,
        Call::UpdateFound(item, _) if item.information_only => match user {
            true => InformationOnly,
            false => Choose(Choice::Dismiss),
        },
        Call::UpdateFound(_, Stage::Installing) => QuitAndInstall,
        Call::UpdateFound(_, _) => match user {
            true => Choose(Choice::Install),
            false => Choose(Choice::Dismiss),
        },
        Call::UpdateNotFound => match user {
            true => UpToDate,
            false => Acknowledge,
        },
        Call::UpdaterError => match user {
            true => ShowError,
            false => Acknowledge,
        },
        Call::ReadyToInstall => match user {
            true => QuitAndInstall,
            false => Choose(Choice::Dismiss),
        },
        Call::InstalledAndRelaunched => Acknowledge,
        Call::InFocus => Focus,
        Call::DismissInstallation => CloseAlert,
    }
}

/// Why a background "update found" call came, for the log.
pub fn why_found(item: Item, stage: Stage) -> &'static str {
    if item.information_only {
        "an information-only update"
    } else if stage == Stage::Installing {
        "an update has waited to install"
    } else if item.critical {
        "a critical update"
    } else if item.major_upgrade {
        "a major upgrade"
    } else {
        "an update Sparkle can't install alone (it may need a password)"
    }
}

/// The "Quit and Install" alert's title and text, `programs` being the
/// programs running in the windows.
pub fn quit_and_install_text(version: Option<&str>, programs: &[String]) -> (String, String) {
    let product = crate::PRODUCT_NAME;
    let title = match version {
        Some(v) => format!("{product} {v} is ready to install"),
        None => format!("An update to {product} is ready to install"),
    };
    let text = match programs {
        [] => format!("It installs when {product} quits."),
        [one] => format!("Quit and Install will end {one}."),
        [first, rest @ ..] => {
            let n = rest.len();
            format!(
                "Quit and Install will end {first} and {n} other program{}.",
                if n == 1 { "" } else { "s" }
            )
        }
    };
    (title, text)
}

/// `update.log` grows to this, then loses its oldest half.
const LOG_CAP: u64 = 256 * 1024;

/// `update.log` in the logs directory.
pub fn log_path(home: &Path) -> PathBuf {
    crate::crash::logs_dir(home).join("update.log")
}

/// One line in `update.log`; nothing is shown.
pub fn log(msg: &str) {
    let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) else {
        return;
    };
    let _ = append_capped(
        &log_path(Path::new(&home)),
        &format!("{} {msg}\n", crate::crash::local_time()),
        LOG_CAP,
    );
}

/// Append `line` to `path`, first dropping the oldest half of the file
/// (whole lines) once it is past `cap` bytes.
fn append_capped(path: &Path, line: &str, cap: u64) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > cap) {
        let text = std::fs::read(path)?;
        let half = text.len() / 2;
        let start = text[half..].iter().position(|&b| b == b'\n').map_or(text.len(), |i| half + i + 1);
        std::fs::write(path, &text[start..])?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(line.as_bytes())
}

thread_local! {
    /// The updater's state for test dumps: `off: <why>`, `on`, `checking`,
    /// `found`, `none`, `ready`, `error: …`.
    static STATUS: RefCell<String> = RefCell::new("off: not started".into());
}

/// The dump's `update` field.
pub fn status() -> String {
    STATUS.with(|s| s.borrow().clone())
}

pub(crate) fn set_status(s: impl Into<String>) {
    STATUS.with(|st| *st.borrow_mut() = s.into());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> Context {
        Context {
            role: Role::App,
            primary: true,
            bundle_id: Some(crate::BUNDLE_ID.into()),
            team_id: Some("ABCDE12345".into()),
            dogfood: false,
            version: "0.4.0".into(),
            setting: true,
            public_key: true,
            framework: true,
            test: false,
            test_feed: false,
        }
    }

    #[test]
    fn decide_turns_off_for_each_reason() {
        assert_eq!(decide(&on()), Decision::On);
        let pin = Role::Pin(crate::PinInfo {
            id: "x".into(),
            bundle: "/p.app".into(),
            app_bundle: "/a.app".into(),
            stub_version: "1".into(),
        });
        type Change = Box<dyn Fn(&mut Context)>;
        let cases: Vec<(&str, Change)> = vec![
            ("pin", Box::new(move |c| c.role = pin.clone())),
            ("bare", Box::new(|c| c.role = Role::Bare)),
            ("second process", Box::new(|c| c.primary = false)),
            ("other bundle id", Box::new(|c| c.bundle_id = Some("red.forbes.decline".into()))),
            ("no bundle id", Box::new(|c| c.bundle_id = None)),
            ("dogfood", Box::new(|c| c.dogfood = true)),
            ("nightly", Box::new(|c| c.version = "0.4.0-nightly.20261029".into())),
            ("dev", Box::new(|c| c.version = "0.4.0-dev".into())),
            ("ad-hoc", Box::new(|c| c.team_id = None)),
            ("setting", Box::new(|c| c.setting = false)),
            ("key", Box::new(|c| c.public_key = false)),
            ("framework", Box::new(|c| c.framework = false)),
            ("test without feed", Box::new(|c| c.test = true)),
        ];
        for (name, change) in cases {
            let mut c = on();
            change(&mut c);
            assert!(matches!(decide(&c), Decision::Off(_)), "{name}");
        }
        // The test feed: ad-hoc builds may check, and only then.
        let mut c = on();
        c.test = true;
        c.test_feed = true;
        c.team_id = None;
        assert_eq!(decide(&c), Decision::On);
        // A dogfood build stays off even with the test feed.
        c.dogfood = true;
        assert!(matches!(decide(&c), Decision::Off("a dogfood build")));
    }

    #[test]
    fn public_keys() {
        assert!(is_public_key("8B6SlbA2LGu5FM2qRfGhvpwdW10UNYCYjXPfDtJPk74="));
        assert!(is_public_key(include_str!("../../../packaging/sparkle-public-key")));
        assert!(!is_public_key(""));
        assert!(!is_public_key("8B6SlbA2LGu5FM2qRfGhvpwdW10UNYCYjXPfDtJPk7=="));
        assert!(!is_public_key("8B6SlbA2LGu5FM2qRfGhvpwdW10UNYCYjXPfDtJPk7"));
        assert!(!is_public_key("8B6SlbA2LGu5FM2qRfGhvpwdW10UNYCYjXPfDtJP!74="));
    }

    #[test]
    fn one_lock_at_a_time() {
        let dir = std::env::temp_dir().join(format!("td-update-lock-{}", std::process::id()));
        let path = dir.join("Caches/updater.lock");
        let first = take_lock(&path).expect("first takes it");
        assert!(take_lock(&path).is_none(), "second can't");
        drop(first);
        assert!(take_lock(&path).is_some(), "free again once the first is gone");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_answer_table() {
        use Answer::*;
        let plain = Item::default();
        let info = Item {
            information_only: true,
            ..Item::default()
        };
        let critical = Item {
            critical: true,
            ..Item::default()
        };
        let found = |item, stage| Call::UpdateFound(item, stage);
        // (call, user asked, answer)
        let rows = [
            (Call::PermissionRequest, false, Permission { automatic_checks: true }),
            (Call::UserInitiatedCheck, true, Nothing),
            (found(plain, Stage::NotDownloaded), false, Choose(Choice::Dismiss)),
            (found(plain, Stage::Downloaded), false, Choose(Choice::Dismiss)),
            (found(critical, Stage::NotDownloaded), false, Choose(Choice::Dismiss)),
            (found(info, Stage::NotDownloaded), false, Choose(Choice::Dismiss)),
            // Waited a week (decided 29 Oct 2026, open question 1).
            (found(plain, Stage::Installing), false, QuitAndInstall),
            (found(info, Stage::Installing), false, Choose(Choice::Dismiss)),
            (found(plain, Stage::NotDownloaded), true, Choose(Choice::Install)),
            (found(plain, Stage::Downloaded), true, Choose(Choice::Install)),
            (found(plain, Stage::Installing), true, QuitAndInstall),
            (found(info, Stage::NotDownloaded), true, InformationOnly),
            (Call::ReleaseNotes, false, Nothing),
            (Call::ReleaseNotes, true, Nothing),
            (Call::UpdateNotFound, true, UpToDate),
            (Call::UpdateNotFound, false, Acknowledge),
            (Call::UpdaterError, true, ShowError),
            (Call::UpdaterError, false, Acknowledge),
            (Call::Progress, false, Nothing),
            (Call::Progress, true, Nothing),
            (Call::ReadyToInstall, false, Choose(Choice::Dismiss)),
            (Call::ReadyToInstall, true, QuitAndInstall),
            (Call::Installing, true, Nothing),
            (Call::InstalledAndRelaunched, false, Acknowledge),
            (Call::InstalledAndRelaunched, true, Acknowledge),
            (Call::InFocus, true, Focus),
            (Call::DismissInstallation, false, CloseAlert),
            (Call::DismissInstallation, true, CloseAlert),
        ];
        for (call, user, want) in rows {
            assert_eq!(answer(call, user, true), want, "{call:?} user={user}");
        }
        assert_eq!(
            answer(Call::PermissionRequest, false, false),
            Permission { automatic_checks: false }
        );
        assert_eq!(why_found(plain, Stage::Installing), "an update has waited to install");
        assert_eq!(why_found(critical, Stage::Downloaded), "a critical update");
    }

    #[test]
    fn quit_and_install_says_what_ends() {
        let p = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (title, text) = quit_and_install_text(Some("0.5.0"), &p(&["top", "vim", "less"]));
        assert_eq!(title, "Terminal Decline 0.5.0 is ready to install");
        assert_eq!(text, "Quit and Install will end top and 2 other programs.");
        assert_eq!(
            quit_and_install_text(None, &p(&["top", "vim"])).1,
            "Quit and Install will end top and 1 other program."
        );
        assert_eq!(quit_and_install_text(None, &p(&["top"])).1, "Quit and Install will end top.");
        assert_eq!(quit_and_install_text(None, &[]).1, "It installs when Terminal Decline quits.");
    }

    #[test]
    fn the_log_drops_its_oldest_half() {
        let dir = std::env::temp_dir().join(format!("td-update-log-{}", std::process::id()));
        let path = dir.join("update.log");
        let _ = std::fs::remove_dir_all(&dir);
        for i in 0..100 {
            append_capped(&path, &format!("line {i:03}\n"), 200).unwrap();
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.len() <= 200 + 9, "{}", text.len());
        assert!(text.ends_with("line 099\n"));
        assert!(text.lines().all(|l| l.starts_with("line ") && l.len() == 8), "{text}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
