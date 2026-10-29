//! P-0's minimal stub, kept for the signing spike (crates/term-pin/spike):
//! reads the pin's Info.plist, takes the app from the override file or
//! `~/Applications`, checks the library's signature, loads it and calls
//! `td_pin_main_v1`. `TD_PIN_SPIKE_TIMES=1` prints how long each step took;
//! `TD_PIN_SPIKE_DRY=1` stops before the call; `TD_PIN_SPIKE_FD=1` loads
//! the library through `/dev/fd/N`.

use std::ffi::{CString, c_char, c_int};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::time::Instant;

use term_pin::abi::{self, PinInfoV1};
use term_pin::cf::MainBundle;
use term_pin::sec;

unsafe extern "C" {
    fn _NSGetArgc() -> *mut c_int;
    fn _NSGetArgv() -> *mut *mut *mut c_char;
}

fn fail(msg: &str) -> ! {
    eprintln!("terminal-decline-pin: {msg}");
    std::process::exit(1)
}

fn leak(b: &[u8]) -> *const c_char {
    CString::new(b).unwrap().into_raw()
}

fn main() {
    let t0 = Instant::now();
    let times = std::env::var_os("TD_PIN_SPIKE_TIMES").is_some();
    let bundle = MainBundle::get().unwrap_or_else(|| fail("no bundle"));
    let id = bundle.string("TDPinId").unwrap_or_else(|| fail("no TDPinId"));
    let req = bundle
        .string("TDLibraryRequirement")
        .unwrap_or_else(|| fail("no TDLibraryRequirement"));
    let pin = bundle.path().unwrap_or_else(|| fail("no bundle path"));
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| fail("no HOME")));
    let app = std::fs::read_to_string(home.join(".config/terminal-decline/pin-app"))
        .map(|s| PathBuf::from(s.trim_end()))
        .unwrap_or_else(|_| home.join("Applications/Terminal Decline.app"));
    let lib = app.join("Contents/Frameworks").join(abi::LIBRARY);
    let t1 = Instant::now();
    if let Err(e) = sec::check(&lib, &req) {
        fail(&format!("{}: {e}", lib.display()));
    }
    let t2 = Instant::now();
    let path = if std::env::var_os("TD_PIN_SPIKE_FD").is_some() {
        // Is the checked file loadable by descriptor? (§3.4)
        let f = std::fs::File::open(&lib).unwrap_or_else(|e| fail(&format!("open: {e}")));
        let p = format!("/dev/fd/{}", std::os::fd::AsRawFd::as_raw_fd(&f));
        std::mem::forget(f);
        CString::new(p).unwrap()
    } else {
        CString::new(lib.as_os_str().as_bytes()).unwrap()
    };
    // SAFETY: a NUL-terminated path.
    let h = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if h.is_null() {
        // SAFETY: dlerror's string is valid until the next dl call.
        let e = unsafe { std::ffi::CStr::from_ptr(libc::dlerror()) };
        fail(&format!("dlopen {path:?}: {}", e.to_string_lossy()));
    }
    // SAFETY: a live handle and a NUL-terminated name.
    let f = unsafe { libc::dlsym(h, abi::entry_name(1).as_ptr().cast()) };
    if f.is_null() {
        fail("no td_pin_main_v1");
    }
    let t3 = Instant::now();
    if times {
        let ms = |a: Instant, b: Instant| (b - a).as_secs_f64() * 1e3;
        eprintln!(
            "spike: plist {:.2} ms, check {:.2} ms, dlopen {:.2} ms",
            ms(t0, t1),
            ms(t1, t2),
            ms(t2, t3)
        );
    }
    if std::env::var_os("TD_PIN_SPIKE_DRY").is_some() {
        std::process::exit(0);
    }
    let info = PinInfoV1 {
        size: size_of::<PinInfoV1>() as u32,
        abi: abi::ABI_V1,
        pin_id: leak(id.as_bytes()),
        pin_bundle: leak(pin.as_os_str().as_bytes()),
        app_bundle: leak(app.as_os_str().as_bytes()),
        stub_version: leak(bundle.string("CFBundleShortVersionString").unwrap_or_default().as_bytes()),
        // SAFETY: the process's own argument vector, alive for its life.
        argc: unsafe { *_NSGetArgc() },
        argv: unsafe { *_NSGetArgv() } as *const *const c_char,
    };
    // SAFETY: the symbol has the frozen v1 signature.
    let main: abi::PinMainV1 = unsafe { std::mem::transmute(f) };
    std::process::exit(unsafe { main(&info) });
}
