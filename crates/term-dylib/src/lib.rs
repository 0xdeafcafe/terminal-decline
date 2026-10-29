//! `libterminal_decline.dylib`: all of Terminal Decline (term-app's
//! library), with the two entry points of
//! `crates/term-pin/abi/terminal_decline_pin.h` (docs/design/pins.md §2.5,
//! §3.5). These are the only symbols it exports.
//!
//! - `td_app_main_v1`: called by the app's launcher,
//!   `Contents/MacOS/terminal-decline`. Not frozen.
//! - `td_pin_main_v1`: called by a pin's stub. Frozen from 0.1.0: kept, and
//!   never changed, in every later library.
//!
//! No panic crosses into C: each entry catches it and returns 70
//! (`EX_SOFTWARE`). Panics on the main thread once the app is running
//! still abort after their crash log (term-app's `crash` module).

use std::ffi::{CStr, OsStr, c_char, c_int};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use term_pin::abi::{ABI_V1, PinInfoV1};
use terminal_decline_app::{PinInfo, Role};

/// What an entry returns after a panic, or when it was called wrong.
const SOFTWARE: c_int = 70;

/// Run the app in `role`, catching a panic.
fn guarded(role: Role, args: Vec<String>) -> c_int {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| terminal_decline_app::run(role, args))).unwrap_or(SOFTWARE)
}

/// The process's arguments without the program name. Not UTF-8: replaced
/// characters, as an argument parser would refuse them anyway.
///
/// # Safety
/// `argv` has `argc` valid C strings (null entries are skipped).
unsafe fn arguments(argc: c_int, argv: *const *const c_char) -> Vec<String> {
    if argv.is_null() {
        return Vec::new();
    }
    (1..argc.max(0) as usize)
        .filter_map(|i| {
            let p = unsafe { *argv.add(i) };
            (!p.is_null()).then(|| String::from_utf8_lossy(unsafe { CStr::from_ptr(p) }.to_bytes()).into_owned())
        })
        .collect()
}

/// # Safety
/// `p` is null or a valid C string.
unsafe fn bytes(p: *const c_char) -> Option<&'static [u8]> {
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_bytes())
}

/// The stub's struct as a [`PinInfo`] and the pin's arguments.
///
/// # Safety
/// `info` is null or points at a `td_pin_info_v1` whose pointers are valid
/// for the life of the process (the header's promise).
unsafe fn pin_info(info: *const PinInfoV1) -> Result<(PinInfo, Vec<String>), String> {
    if info.is_null() {
        return Err("no pin info".into());
    }
    // `size` first: a mismatched header could make the rest unreadable.
    let size = unsafe { (*info).size } as usize;
    if size != size_of::<PinInfoV1>() {
        return Err(format!("pin info is {size} bytes, expected {}", size_of::<PinInfoV1>()));
    }
    let info = unsafe { &*info };
    if info.abi != ABI_V1 {
        return Err(format!("pin info is version {}, expected {ABI_V1}", info.abi));
    }
    let path = |p, what: &str| -> Result<PathBuf, String> {
        match unsafe { bytes(p) } {
            Some(b) if !b.is_empty() => Ok(PathBuf::from(OsStr::from_bytes(b))),
            _ => Err(format!("pin info has no {what}")),
        }
    };
    let id = match unsafe { bytes(info.pin_id) }.map(std::str::from_utf8) {
        Some(Ok(id)) if !id.is_empty() && !id.contains('/') && id != ".." => id.to_string(),
        _ => return Err("pin info has no usable pin id".into()),
    };
    let pin = PinInfo {
        id,
        bundle: path(info.pin_bundle, "pin bundle")?,
        app_bundle: path(info.app_bundle, "app bundle")?,
        stub_version: unsafe { bytes(info.stub_version) }
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default(),
    };
    Ok((pin, unsafe { arguments(info.argc, info.argv) }))
}

/// `int td_app_main_v1(int argc, const char *const *argv)`.
///
/// # Safety
/// `argv` has `argc` valid C strings, as `main` got them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn td_app_main_v1(argc: c_int, argv: *const *const c_char) -> c_int {
    let args = match std::panic::catch_unwind(|| unsafe { arguments(argc, argv) }) {
        Ok(a) => a,
        Err(_) => return SOFTWARE,
    };
    guarded(Role::App, args)
}

/// `int td_pin_main_v1(const struct td_pin_info_v1 *info)`.
///
/// # Safety
/// `info` points at a `td_pin_info_v1` as the header describes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn td_pin_main_v1(info: *const PinInfoV1) -> c_int {
    match std::panic::catch_unwind(|| unsafe { pin_info(info) }) {
        Ok(Ok((pin, args))) => guarded(Role::Pin(pin), args),
        Ok(Err(e)) => {
            eprintln!("terminal-decline: {e}; this pin and this Terminal Decline don't match, run `terminal-decline pin --repair`");
            SOFTWARE
        }
        Err(_) => SOFTWARE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(pin_id: &CStr, argv: &[*const c_char]) -> PinInfoV1 {
        PinInfoV1 {
            size: size_of::<PinInfoV1>() as u32,
            abi: ABI_V1,
            pin_id: pin_id.as_ptr(),
            pin_bundle: c"/Users/me/Applications/Rush.app".as_ptr(),
            app_bundle: c"/Applications/Terminal Decline.app".as_ptr(),
            stub_version: c"1".as_ptr(),
            argc: argv.len() as c_int,
            argv: argv.as_ptr(),
        }
    }

    #[test]
    fn pin_info_becomes_the_pin_role() {
        let argv = [
            c"/Users/me/Applications/Rush.app/Contents/MacOS/rush".as_ptr(),
            c"-psn_0_1".as_ptr(),
        ];
        let i = info(c"rush", &argv);
        let (pin, args) = unsafe { pin_info(&i) }.unwrap();
        assert_eq!(
            pin,
            PinInfo {
                id: "rush".into(),
                bundle: "/Users/me/Applications/Rush.app".into(),
                app_bundle: "/Applications/Terminal Decline.app".into(),
                stub_version: "1".into(),
            }
        );
        assert_eq!(args, ["-psn_0_1"]);
    }

    #[test]
    fn wrong_pin_info_is_refused() {
        assert!(unsafe { pin_info(std::ptr::null()) }.is_err());
        let argv = [c"rush".as_ptr()];
        let mut i = info(c"rush", &argv);
        i.size = 48;
        assert!(unsafe { pin_info(&i) }.unwrap_err().contains("48 bytes"));
        let mut i = info(c"rush", &argv);
        i.abi = 2;
        assert!(unsafe { pin_info(&i) }.unwrap_err().contains("version 2"));
        for bad in [c"", c"a/b", c".."] {
            assert!(unsafe { pin_info(&info(bad, &argv)) }.is_err(), "{bad:?}");
        }
        let mut i = info(c"rush", &argv);
        i.app_bundle = std::ptr::null();
        assert!(unsafe { pin_info(&i) }.unwrap_err().contains("app bundle"));
    }

    #[test]
    fn arguments_skip_the_program_name() {
        let argv = [c"terminal-decline".as_ptr(), c"-e".as_ptr(), c"/bin/sh".as_ptr()];
        assert_eq!(unsafe { arguments(3, argv.as_ptr()) }, ["-e", "/bin/sh"]);
        assert!(unsafe { arguments(0, argv.as_ptr()) }.is_empty());
        assert!(unsafe { arguments(2, std::ptr::null()) }.is_empty());
    }
}
