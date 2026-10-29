//! The pin stub (docs/design/pins.md §3.2): read the pin's Info.plist,
//! find the app, check its library, load it and call the newest interface
//! both know.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::abi::{self, PinInfoV1};
use crate::cf::MainBundle;
use crate::find::{self, Candidate};
use crate::message::Failure;
use crate::{alert, sec};

/// What the stub reads from its pin's Info.plist (§3.1), written once
/// when the pin was made.
#[derive(Debug, Clone)]
pub struct Pin {
    /// `TDPinAbi`: the interface version the pin was made with.
    pub abi: u32,
    pub id: String,
    pub app_bundle_id: String,
    pub requirement: String,
    /// `CFBundleShortVersionString`, the stub's version.
    pub version: String,
    pub bundle: PathBuf,
}

/// The pin's display name, for messages: its `CFBundleName`, else the
/// bundle's file name, else "This pin".
pub fn name(bundle: Option<&MainBundle>) -> String {
    bundle
        .and_then(|b| {
            b.string("CFBundleName")
                .or_else(|| b.path()?.file_stem().map(|s| s.to_string_lossy().into_owned()))
        })
        .unwrap_or_else(|| "This pin".into())
}

/// Reads the pin's keys. `Err` names what is missing or wrong.
pub fn read_pin(bundle: &MainBundle) -> Result<Pin, String> {
    let s = |key: &str| {
        bundle
            .string(key)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("no {key} in its Info.plist"))
    };
    let abi = crate::cf::to_i64(bundle.value("TDPinAbi"))
        .and_then(|v| u32::try_from(v).ok())
        .filter(|&v| v >= 1)
        .ok_or("no TDPinAbi in its Info.plist")?;
    Ok(Pin {
        abi,
        id: s("TDPinId")?,
        app_bundle_id: s("TDAppBundleId")?,
        requirement: s("TDLibraryRequirement")?,
        version: bundle.string("CFBundleShortVersionString").unwrap_or_default(),
        bundle: bundle.path().ok_or("its bundle has no path")?,
    })
}

/// A loaded library and the newest entry point found in it.
struct Loaded {
    version: u32,
    entry: *mut c_void,
}

fn dlerror() -> String {
    // SAFETY: dlerror's string is valid until the next dl call.
    unsafe {
        let e = libc::dlerror();
        if e.is_null() {
            "unknown error".into()
        } else {
            CStr::from_ptr(e).to_string_lossy().into_owned()
        }
    }
}

/// Checks and loads the library in `app`, then looks up the entry points
/// from `latest` down to 1.
fn load(app: &Path, pin: &Pin, latest: u32) -> Result<Loaded, Failure> {
    let library = find::library(app, &pin.app_bundle_id).map_err(|_| Failure::NotInstalled)?;
    let shown = library.display().to_string();
    sec::check(&library, &pin.requirement).map_err(|why| Failure::CheckFailed {
        library: shown.clone(),
        why,
    })?;
    let c = CString::new(library.as_os_str().as_bytes()).map_err(|_| Failure::NotInstalled)?;
    // SAFETY: a NUL-terminated absolute path.
    let h = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if h.is_null() {
        return Err(Failure::LoadFailed {
            library: shown,
            why: dlerror(),
        });
    }
    for (version, name) in abi::entry_names(latest) {
        // SAFETY: a live handle and a NUL-terminated name.
        let entry = unsafe { libc::dlsym(h, name.as_ptr().cast()) };
        if !entry.is_null() {
            return Ok(Loaded { version, entry });
        }
    }
    // SAFETY: the handle from dlopen above, not used again.
    unsafe { libc::dlclose(h) };
    Err(Failure::NeedsNewer)
}

unsafe extern "C" {
    fn _NSGetArgc() -> *mut c_int;
    fn _NSGetArgv() -> *mut *mut *mut c_char;
}

fn leak(b: &[u8]) -> *const c_char {
    // Paths can't hold NUL; anything else that does becomes empty.
    CString::new(b).unwrap_or_default().into_raw()
}

/// Calls the entry point; returns the exit status.
fn call(loaded: &Loaded, pin: &Pin, app: &Path) -> c_int {
    match loaded.version {
        1 => {
            let info = Box::leak(Box::new(PinInfoV1 {
                size: size_of::<PinInfoV1>() as u32,
                abi: abi::ABI_V1,
                pin_id: leak(pin.id.as_bytes()),
                pin_bundle: leak(pin.bundle.as_os_str().as_bytes()),
                app_bundle: leak(app.as_os_str().as_bytes()),
                stub_version: leak(pin.version.as_bytes()),
                // SAFETY: the process's own argument vector, alive for its life.
                argc: unsafe { *_NSGetArgc() },
                argv: unsafe { *_NSGetArgv() } as *const *const c_char,
            }));
            // SAFETY: td_pin_main_v1 has the frozen v1 signature.
            let main: abi::PinMainV1 = unsafe { std::mem::transmute(loaded.entry) };
            unsafe { main(info) }
        }
        // entry_names only yields versions this stub knows.
        _ => unreachable!(),
    }
}

/// The stub's whole run; returns the exit status (1 when it couldn't
/// start, after saying why).
pub fn run() -> i32 {
    let bundle = MainBundle::get();
    let name = name(bundle.as_ref());
    let pin = match bundle
        .as_ref()
        .ok_or_else(|| "it isn't inside an app bundle".to_string())
        .and_then(read_pin)
    {
        Ok(p) => p,
        Err(why) => return fail(&name, &Failure::Damaged(why), &[]),
    };
    let latest = pin.abi.min(abi::ABI_LATEST);
    let home = find::home();
    let mut notes = Vec::new();
    let candidates = find::candidates(home.as_deref(), &pin.app_bundle_id, find::launch_services, &mut notes);
    let mut worst = Failure::NotInstalled;
    for Candidate { app, .. } in &candidates {
        match load(app, &pin, latest) {
            Ok(loaded) => return call(&loaded, &pin, app),
            Err(f) => {
                notes.push(format!("{}: {}", app.display(), detail(app, &pin, &f)));
                worst = worst.max(f);
            }
        }
    }
    if candidates.is_empty() {
        notes.push(format!(
            "looked in: the override file, /Applications, ~/Applications, and LaunchServices for {}",
            pin.app_bundle_id
        ));
    }
    fail(&name, &worst, &notes)
}

/// One line per candidate for stderr.
fn detail(app: &Path, pin: &Pin, f: &Failure) -> String {
    match f {
        Failure::NotInstalled => find::library(app, &pin.app_bundle_id).err().unwrap_or_default(),
        Failure::CheckFailed { why, .. } | Failure::LoadFailed { why, .. } => why.clone(),
        Failure::NeedsNewer => format!("no td_pin_main_v1..v{}", pin.abi.min(abi::ABI_LATEST)),
        Failure::Damaged(why) => why.clone(),
    }
}

fn fail(name: &str, f: &Failure, notes: &[String]) -> i32 {
    let text = f.text(name);
    eprintln!("{name}: {text}");
    for n in notes {
        eprintln!("  {n}");
    }
    alert::show(name, &text);
    1
}
