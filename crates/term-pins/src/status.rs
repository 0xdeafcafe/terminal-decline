//! What state each pin is in, worked out without starting it
//! (docs/design/pins.md §5.1): for `pin --list`, `--repair`, `--update`
//! and the migration notice.

use std::path::{Path, PathBuf};

use term_pin::abi;
use term_pin::cf::InfoPlist;

use super::record::{Method, Record};
use super::{ID_PREFIX, OLD_ID_PREFIX};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// A stub pin; `dev` when its requirement is the ad-hoc form.
    Ok { dev: bool },
    /// A clone pin from before stubs (`TDPinAbi` absent): `--repair`
    /// makes it a stub pin.
    Old,
    /// A clone made by a bare binary (§5.6), kept as one.
    Clone,
    /// Its interface version is newer than this library's.
    NeedsNewer(u32),
    /// Its signature or Info.plist is broken.
    Damaged(String),
    /// Not where its settings say; LaunchServices knows it here.
    Moved(PathBuf),
    /// Neither.
    Missing,
}

impl Status {
    /// The words `--list` shows.
    pub fn label(&self) -> String {
        match self {
            Status::Ok { dev: false } => "ok".into(),
            Status::Ok { dev: true } => "ok (dev pin)".into(),
            Status::Old => "old: run --repair".into(),
            Status::Clone => "clone".into(),
            Status::NeedsNewer(_) => "needs a newer app".into(),
            Status::Damaged(_) => "damaged".into(),
            Status::Moved(_) => "pin moved: --repair".into(),
            Status::Missing => "missing".into(),
        }
    }

    /// A short word for scripts (`--list --json`).
    pub fn key(&self) -> &'static str {
        match self {
            Status::Ok { .. } => "ok",
            Status::Old => "old",
            Status::Clone => "clone",
            Status::NeedsNewer(_) => "needs-newer-app",
            Status::Damaged(_) => "damaged",
            Status::Moved(_) => "moved",
            Status::Missing => "missing",
        }
    }
}

/// Whether the bundle at `app` is a stub pin (its Info.plist has
/// `TDPinAbi`).
pub fn is_stub_pin(app: &Path) -> bool {
    InfoPlist::read(app).is_some_and(|p| p.integer("TDPinAbi").is_some())
}

/// The state of pin `id`, whose settings say it is at `app`. `lookup`
/// asks LaunchServices for a bundle id's apps
/// ([`term_pin::find::launch_services`]).
pub fn of(id: &str, app: &Path, record: Option<&Record>, lookup: &dyn Fn(&str) -> Vec<PathBuf>) -> Status {
    if !app.join("Contents").is_dir() {
        let elsewhere = [ID_PREFIX, OLD_ID_PREFIX]
            .iter()
            .flat_map(|prefix| lookup(&format!("{prefix}{id}")))
            .find(|p| p != app && p.join("Contents").is_dir() && !p.to_string_lossy().contains("/.Trash/"));
        return match elsewhere {
            Some(p) => Status::Moved(p),
            None => Status::Missing,
        };
    }
    let Some(info) = InfoPlist::read(app) else {
        return Status::Damaged("its Info.plist can't be read".into());
    };
    let Some(version) = info.integer("TDPinAbi") else {
        return match record.map(|r| r.method) {
            Some(Method::Clone) => Status::Clone,
            _ => Status::Old,
        };
    };
    if version < 1 || version > i64::from(abi::ABI_LATEST) {
        return Status::NeedsNewer(version.max(0) as u32);
    }
    let bundle_id = info.string("CFBundleIdentifier").unwrap_or_default();
    if let Err(e) = term_pin::sec::check(app, &format!("identifier \"{bundle_id}\"")) {
        return Status::Damaged(e);
    }
    let dev = info.string("TDLibraryRequirement").is_some_and(|r| r == abi::dev_requirement());
    Status::Ok { dev }
}
