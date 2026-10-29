//! Finding Terminal Decline.app (docs/design/pins.md §3.3). The order and
//! the paths are frozen with the stub (FROZEN.md).

use std::ffi::{CStr, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::abi;
use crate::cf::{self, CFTypeRef, InfoPlist, Owned};

/// The app's file name in `/Applications` and `~/Applications`.
pub const APP_NAME: &str = "Terminal Decline.app";

/// The dev override, relative to `HOME`: one line, an absolute app path.
/// `XDG_CONFIG_HOME` is ignored, since a Dock launch doesn't have it.
pub const OVERRIDE_FILE: &str = ".config/terminal-decline/pin-app";

/// Where a candidate came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Override,
    Applications,
    HomeApplications,
    LaunchServices,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The app bundle, real path (hardened `dlopen` refuses relative ones).
    pub app: PathBuf,
    pub source: Source,
}

/// `HOME` from the environment, else the user's home from the password
/// database (a launch from the Dock always has `HOME`, but cheaply safe).
pub fn home() -> Option<PathBuf> {
    if let Some(h) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        return Some(PathBuf::from(h));
    }
    // SAFETY: getpwuid returns a pointer to static storage or null; the
    // directory string is copied before any other call.
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_dir.is_null() {
            return None;
        }
        Some(PathBuf::from(std::ffi::OsStr::from_bytes(CStr::from_ptr((*pw).pw_dir).to_bytes())))
    }
}

/// The app named by the override file's contents: an absolute path, made
/// real. `Err` says why it is refused.
pub fn parse_override(text: &str) -> Result<PathBuf, String> {
    let line = text.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return Err("is empty".into());
    }
    let p = Path::new(line);
    if !p.is_absolute() {
        return Err(format!("names {line:?}, which isn't an absolute path"));
    }
    p.canonicalize().map_err(|e| format!("names {line}: {e}"))
}

/// Whether LaunchServices' copy at `app` is one a pin must never use: in
/// a Trash, on a disk image or other volume, translocated, or still in
/// Downloads.
pub fn skipped_by_launch_services(app: &Path, home: Option<&Path>) -> bool {
    let s = app.as_os_str().as_bytes();
    let has = |needle: &[u8]| s.windows(needle.len()).any(|w| w == needle);
    has(b"/.Trash/")
        || has(b"/.Trashes/")
        || app.starts_with("/Volumes")
        || has(b"/AppTranslocation/")
        || home.is_some_and(|h| app.starts_with(h.join("Downloads")))
}

/// Compares `CFBundleVersion`s numerically, part by part ("1.10" > "1.9");
/// a part that isn't a number counts as 0.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |v: &str| v.split('.').map(|p| p.trim().parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (mut x, mut y) = (parts(a), parts(b));
    let n = x.len().max(y.len());
    x.resize(n, 0);
    y.resize(n, 0);
    x.cmp(&y)
}

/// Every candidate, in the frozen order: the override, `/Applications`,
/// `~/Applications`, then what LaunchServices knows by `bundle_id`
/// (highest build first). Each path appears once. Why a source gave
/// nothing usable goes into `notes`.
pub fn candidates(
    home: Option<&Path>,
    bundle_id: &str,
    launch_services: impl FnOnce(&str) -> Vec<PathBuf>,
    notes: &mut Vec<String>,
) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    let push = |app: PathBuf, source: Source, out: &mut Vec<Candidate>| {
        let app = app.canonicalize().unwrap_or(app);
        if !out.iter().any(|c| c.app == app) {
            out.push(Candidate { app, source });
        }
    };
    if let Some(h) = home {
        let file = h.join(OVERRIDE_FILE);
        if let Ok(text) = std::fs::read_to_string(&file) {
            match parse_override(&text) {
                Ok(app) => push(app, Source::Override, &mut out),
                Err(e) => notes.push(format!("{} {e}", file.display())),
            }
        }
    }
    let global = Path::new("/Applications").join(APP_NAME);
    if global.exists() {
        push(global, Source::Applications, &mut out);
    }
    if let Some(h) = home {
        let local = h.join("Applications").join(APP_NAME);
        if local.exists() {
            push(local, Source::HomeApplications, &mut out);
        }
    }
    let mut found: Vec<(String, PathBuf)> = launch_services(bundle_id)
        .into_iter()
        .filter(|p| {
            let skip = skipped_by_launch_services(p, home);
            if skip {
                notes.push(format!("{}: skipped (Trash, disk image, translocated or Downloads)", p.display()));
            }
            !skip
        })
        .map(|p| (InfoPlist::read(&p).and_then(|i| i.string("CFBundleVersion")).unwrap_or_default(), p))
        .collect();
    found.sort_by(|a, b| compare_versions(&b.0, &a.0));
    for (_, p) in found {
        push(p, Source::LaunchServices, &mut out);
    }
    out
}

/// The library inside `app`, if `app` is Terminal Decline with
/// `bundle_id`. `Err` says what is wrong with it.
pub fn library(app: &Path, bundle_id: &str) -> Result<PathBuf, String> {
    let info = InfoPlist::read(app).ok_or("has no readable Contents/Info.plist")?;
    match info.string("CFBundleIdentifier") {
        Some(id) if id == bundle_id => {}
        Some(id) => return Err(format!("is {id}, not {bundle_id}")),
        None => return Err("has no CFBundleIdentifier".into()),
    }
    let lib = app.join("Contents/Frameworks").join(abi::LIBRARY);
    if !lib.is_file() {
        return Err(format!("has no Contents/Frameworks/{}", abi::LIBRARY));
    }
    Ok(lib)
}

type CopyUrls = unsafe extern "C" fn(CFTypeRef, *mut CFTypeRef) -> CFTypeRef;

/// The apps LaunchServices knows by `bundle_id`. Looked up with `dlsym`:
/// `LSCopyApplicationURLsForBundleIdentifier` is "to be deprecated", and
/// if a later macOS removes it this step finds nothing instead of the stub
/// failing to start.
pub fn launch_services(bundle_id: &str) -> Vec<PathBuf> {
    // SAFETY: a NUL-terminated framework path and symbol name; the
    // function has the documented signature; its array is released by
    // Owned and its URLs only borrowed from it.
    unsafe {
        let h = libc::dlopen(
            c"/System/Library/Frameworks/CoreServices.framework/CoreServices".as_ptr(),
            libc::RTLD_LAZY | libc::RTLD_LOCAL,
        );
        if h.is_null() {
            return Vec::new();
        }
        let f: *mut c_void = libc::dlsym(h, c"LSCopyApplicationURLsForBundleIdentifier".as_ptr());
        if f.is_null() {
            return Vec::new();
        }
        let f: CopyUrls = std::mem::transmute(f);
        let id = cf::string(bundle_id);
        let Some(urls) = Owned::new(f(id.get(), std::ptr::null_mut())) else {
            return Vec::new();
        };
        cf::array(urls.get()).into_iter().filter_map(cf::url_path).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn override_must_be_absolute() {
        assert!(parse_override("").is_err());
        assert!(parse_override("\n").is_err());
        assert!(
            parse_override("Applications/Terminal Decline.app\n")
                .unwrap_err()
                .contains("absolute")
        );
        assert_eq!(parse_override("/\n").unwrap(), PathBuf::from("/"));
        assert!(
            parse_override("/nonexistent/Terminal Decline.app")
                .unwrap_err()
                .contains("/nonexistent")
        );
    }

    #[test]
    fn launch_services_skips() {
        let home = Path::new("/Users/me");
        let skip = |p: &str| skipped_by_launch_services(Path::new(p), Some(home));
        assert!(skip("/Users/me/.Trash/Terminal Decline.app"));
        assert!(skip("/Volumes/Terminal Decline/Terminal Decline.app"));
        assert!(skip("/Volumes/Data/.Trashes/501/Terminal Decline.app"));
        assert!(skip("/private/var/folders/x/AppTranslocation/ABC/d/Terminal Decline.app"));
        assert!(skip("/Users/me/Downloads/Terminal Decline.app"));
        assert!(!skip("/Users/me/Tools/Terminal Decline.app"));
        assert!(!skip("/Applications/Utilities/Terminal Decline.app"));
        assert!(!skip("/Users/me/DownloadsArchive/Terminal Decline.app"));
    }

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(compare_versions("1.10", "1.9"), Ordering::Greater);
        assert_eq!(compare_versions("1234", "999"), Ordering::Greater);
        assert_eq!(compare_versions("1.0", "1"), Ordering::Equal);
        assert_eq!(compare_versions("", "1"), Ordering::Less);
    }

    #[test]
    fn order_and_no_duplicates() {
        let dir = std::env::temp_dir().join(format!("term-pin-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let home = dir.join("home");
        let other = dir.join("other/Terminal Decline.app");
        std::fs::create_dir_all(home.join("Applications").join(APP_NAME)).unwrap();
        std::fs::create_dir_all(home.join(".config/terminal-decline")).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(home.join(OVERRIDE_FILE), format!("{}\n", other.display())).unwrap();
        let mut notes = Vec::new();
        let ls = |_: &str| vec![home.join("Applications").join(APP_NAME), home.join(".Trash").join(APP_NAME)];
        let found = candidates(Some(&home), "x.test", ls, &mut notes);
        let found: Vec<_> = found.into_iter().filter(|c| c.source != Source::Applications).collect();
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(
            found[0],
            Candidate {
                app: other.canonicalize().unwrap(),
                source: Source::Override
            }
        );
        assert_eq!(found[1].source, Source::HomeApplications);
        assert_eq!(notes.len(), 1, "{notes:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
