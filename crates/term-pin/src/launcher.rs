//! The app's launcher, `Contents/MacOS/terminal-decline` (docs/design/
//! pins.md §2.2): loads `../Frameworks/libterminal_decline.dylib` and calls
//! `td_app_main_v1`. No signature check of its own: in release builds it
//! is hardened, so macOS's library validation already requires the
//! library to be signed by the same team.

use std::ffi::{CString, c_char, c_int};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use crate::{abi, alert, message};

unsafe extern "C" {
    fn _NSGetArgc() -> *mut c_int;
    fn _NSGetArgv() -> *mut *mut *mut c_char;
}

/// The library next to the launcher, through any symlinks to it (the
/// cask links `terminal-decline` and `decline` to it; hardened `dlopen`
/// refuses relative paths).
pub fn library_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("can't find its own path: {e}"))?;
    let exe = exe.canonicalize().map_err(|e| format!("{}: {e}", exe.display()))?;
    let contents = exe.parent().and_then(|p| p.parent()).ok_or("isn't inside an app bundle")?;
    Ok(contents.join("Frameworks").join(abi::LIBRARY))
}

/// The launcher's whole run; returns the exit status.
pub fn run() -> i32 {
    let lib = match library_path() {
        Ok(l) => l,
        Err(why) => return fail("Terminal Decline", &why),
    };
    let shown = lib.display().to_string();
    if !lib.is_file() {
        return fail(&shown, "is missing");
    }
    let Ok(c) = CString::new(lib.as_os_str().as_bytes()) else {
        return fail(&shown, "has an unusable path");
    };
    // SAFETY: a NUL-terminated absolute path; a live handle and a
    // NUL-terminated symbol name.
    let entry = unsafe {
        let h = libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if h.is_null() {
            let e = std::ffi::CStr::from_ptr(libc::dlerror()).to_string_lossy().into_owned();
            return fail(&shown, &format!("can't be loaded: {e}"));
        }
        libc::dlsym(h, abi::APP_MAIN_V1.as_ptr())
    };
    if entry.is_null() {
        return fail(&shown, "has no td_app_main_v1");
    }
    // SAFETY: td_app_main_v1's signature; the process's own arguments.
    unsafe {
        let main: abi::AppMainV1 = std::mem::transmute(entry);
        main(*_NSGetArgc(), *_NSGetArgv() as *const *const c_char)
    }
}

fn fail(library: &str, why: &str) -> i32 {
    let text = message::launcher_failure(library, why);
    eprintln!("terminal-decline: {text}");
    // SAFETY: isatty on a descriptor number.
    if unsafe { libc::isatty(2) } == 0 {
        alert::show("Terminal Decline", &text);
    }
    1
}
